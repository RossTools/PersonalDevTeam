use super::*;

#[tokio::test]
async fn comments_and_timeline_paging() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", new("A")).await;
    for i in 0..5 {
        let e = store.add_comment("a", &format!("c{i}"), 2000 + i).await.unwrap().unwrap();
        assert_eq!((e.kind, e.body.as_deref()), (EventKind::Comment, Some(format!("c{i}").as_str())));
    }
    assert!(store.add_comment("nope", "x", 1).await.unwrap().is_none());
    assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().updated_at, 2004);
    let mut seen = vec![];
    let mut cursor = None;
    loop {
        let page = store.list_events("a", cursor.as_deref(), 2, true).await.unwrap().unwrap();
        seen.extend(page.items.iter().map(|e| e.body.clone().unwrap_or_default()));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen, ["c4", "c3", "c2", "c1", "c0", ""]);
    assert_eq!(store.comment_bodies("a").await.unwrap(), ["c0", "c1", "c2", "c3", "c4"]);
    assert!(store.comment_bodies("nope").await.unwrap().is_empty());
    assert!(store.list_events("nope", None, 2, false).await.unwrap().is_none());
    assert!(matches!(store.list_events("a", Some("junk"), 2, false).await, Err(TicketError::Invalid(_))));
}

/// The payloads of the events of `kind` logged about ticket `id`, oldest first (PDT-363).
async fn logged(store: &Store, kind: &str, id: &str) -> Vec<serde_json::Value> {
    let f =
        crate::EventFilter { kind: Some(kind.into()), ticket_id: Some(id.into()), limit: 100, ..Default::default() };
    store.list_log_events(&f).await.unwrap().into_iter().map(|e| e.payload).collect()
}

fn coder_on(key: &str) -> By {
    By::agent(AgentBy {
        session_id: "s1".into(),
        project_id: Some("p".into()),
        persona: Some("Coder".into()),
        from_ticket: Some(key.into()),
        from_ticket_id: Some("from".into()),
    })
}

#[tokio::test]
async fn ticket_created_says_who_made_it() {
    use pdt_core::workflow::events::TICKET_CREATED;
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let mine = ValidNewTicket { labels: vec!["ui".into()], priority: Priority::High, ..new("Mine") };
    add(&store, "u", mine).await;
    let [e] = logged(&store, TICKET_CREATED, "u").await.try_into().unwrap();
    assert_eq!(
        (e["actor"].as_str(), e["title"].as_str(), e["stage"].as_str()),
        (Some("user"), Some("Mine"), Some("draft"))
    );
    assert_eq!((e["labels"].clone(), e["priority"].as_str()), (json!(["ui"]), Some("high")));
    assert_eq!((e["persona"].clone(), e["viaParts"].as_bool()), (json!(null), Some(false)));

    // An agent's: its persona, session and the ticket it was working on, also on the ticket.
    let t = ValidNewTicket { source: TicketSource::Agent, parent: Some("u".into()), ..new("Follow-up") };
    let made = store.create_ticket_by("a", "p", &t, coder_on("PX-9"), 2).await.unwrap();
    assert_eq!(made.summary.source, TicketSource::Agent);
    let by = made.summary.created_by.clone().expect("created by the agent");
    let listed = store.get_ticket_summary("a").await.unwrap().unwrap();
    assert_eq!(listed.created_by.as_ref().and_then(|b| b.persona.as_deref()), Some("Coder"), "on the summary too");
    assert_eq!((by.persona.as_deref(), by.from_ticket.as_deref()), (Some("Coder"), Some("PX-9")));
    let [e] = logged(&store, TICKET_CREATED, "a").await.try_into().unwrap();
    assert_eq!((e["actor"].as_str(), e["persona"].as_str()), (Some("agent"), Some("Coder")));
    assert_eq!((e["sessionId"].as_str(), e["fromTicket"].as_str()), (Some("s1"), Some("PX-9")));
    assert_eq!((e["fromTicketId"].as_str(), e["parentId"].as_str()), (Some("from"), Some("u")));
    assert!(e["parentKey"].as_str().is_some_and(|k| k.ends_with("-1")), "{e}");
    assert!(store.get_ticket("u").await.unwrap().unwrap().summary.created_by.is_none(), "the user's");

    // A workflow's, and parts made with Make into parts.
    store.create_ticket_by("w", "p", &new("Nightly"), Actor::System, 3).await.unwrap();
    assert_eq!(logged(&store, TICKET_CREATED, "w").await[0]["actor"], "workflow");
    let v = ValidParts { parts: vec![("One".into(), String::new())], blocked_by_previous: false };
    let part = store.create_parts("u", &v, 4).await.unwrap().remove(0);
    let [e] = logged(&store, TICKET_CREATED, &part.id).await.try_into().unwrap();
    assert_eq!((e["actor"].as_str(), e["viaParts"].as_bool()), (Some("user"), Some(true)));
}

/// PDT-366: an agent's ticket proposal is held from its making (before any Inbox item exists) and off the lists
/// until it is decided; an agent can't move it on or label it agentic, the user can; leaving Draft releases it and
/// closes its item; its `created` event carries the agent's reason.
#[tokio::test]
async fn a_ticket_proposal_waits_off_the_lists() {
    use pdt_core::inbox::{InboxGroup, InboxItem, InboxKind, InboxStatus};
    use pdt_core::workflow::events::TICKET_CREATED;
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let proposal = |id: &str, ticket: &str| InboxItem {
        id: id.into(),
        kind: InboxKind::TicketProposal,
        group: InboxGroup::TicketProposal,
        status: InboxStatus::Open,
        project_id: Some("p".into()),
        ticket_id: Some(ticket.into()),
        ticket_key: None,
        ticket_title: None,
        attempt_id: None,
        session_id: None,
        run_id: None,
        step_id: None,
        persona_id: None,
        persona_name: None,
        change_request: None,
        title: "Follow-up".into(),
        body: json!({}),
        choices: vec![],
        resolution: serde_json::Value::Null,
        created_at: 1,
        resolved_at: None,
    };
    let t = ValidNewTicket { source: TicketSource::Agent, ..new("Follow-up") };
    let held = |reason| Made { reason, proposal: true, ..Made::default() };
    store.create_ticket_made("a", "p", &t, coder_on("PX-9"), held(Some(" It flakes ")), 1).await.unwrap();
    store.create_ticket_made("b", "p", &t, coder_on("PX-9"), held(None), 1).await.unwrap();
    store.create_ticket_by("c", "p", &t, coder_on("PX-9"), 1).await.unwrap();
    add(&store, "u", new("Mine")).await;
    let [e] = logged(&store, TICKET_CREATED, "a").await.try_into().unwrap();
    assert_eq!(e["reason"], "It flakes");
    assert_eq!(logged(&store, TICKET_CREATED, "b").await[0]["reason"], json!(null));
    // Held from the start, before the workflow puts its item in the Inbox.
    assert!(store.get_ticket_summary("a").await.unwrap().unwrap().proposed);
    assert!(!store.get_ticket_summary("c").await.unwrap().unwrap().proposed, "not held: proposals were off");
    assert!(!store.get_ticket_summary("u").await.unwrap().unwrap().proposed);
    // Listed until the workflow puts it in the Inbox: one it never does stays reachable.
    assert_eq!(ids(&store, in_project()).await.len(), 4, "no item yet: still listed");
    assert!(ids(&store, TicketQuery { proposed: Some(true), ..in_project() }).await.is_empty());

    // At once, before any item: the agent can't move it on or label it agentic; it can edit it or withdraw it.
    let agent = coder_on("PX-9");
    let moved = store.move_ticket_by("a", TicketStage::Todo, &agent, None, 2).await;
    assert!(matches!(moved, Err(TicketError::Untrusted(_))), "{moved:?}");
    let label = ValidPatch { labels: Some(vec![pdt_core::ticket::AGENTIC_LABEL.into()]), ..Default::default() };
    assert!(matches!(store.patch_ticket_by("a", &label, &agent, 2).await, Err(TicketError::Untrusted(_))));
    let todo = ValidPatch { stage: Some(TicketStage::Todo), ..Default::default() };
    assert!(matches!(store.patch_ticket_by("a", &todo, &agent, 2).await, Err(TicketError::Untrusted(_))));
    let edit = ValidPatch { title: Some("Better".into()), ..Default::default() };
    store.patch_ticket_by("a", &edit, &agent, 2).await.unwrap();
    store.add_inbox_item_once(&proposal("ia", "a"), "ticket-proposal:a").await.unwrap();
    store.add_inbox_item_once(&proposal("ib", "b"), "ticket-proposal:b").await.unwrap();
    let mut listed = ids(&store, in_project()).await;
    listed.sort();
    assert_eq!(listed, ["c", "u"], "in the Inbox: off the board and lists");
    let mut only = ids(&store, TicketQuery { proposed: Some(true), ..in_project() }).await;
    only.sort();
    assert_eq!(only, ["a", "b"]);
    store.move_ticket_by("b", TicketStage::WontDo, &agent, None, 2).await.unwrap();
    let b = store.get_inbox_item("ib").await.unwrap().unwrap();
    assert_eq!((b.status, b.resolution), (InboxStatus::Closed, json!({"moved": "wont_do"})));
    assert!(!store.get_ticket_summary("b").await.unwrap().unwrap().proposed);

    // The user moving it out of Draft decides it: the item closes and it is listed again.
    store.move_ticket_by("a", TicketStage::Ready, Actor::User, None, 3).await.unwrap();
    assert_eq!(store.get_inbox_item("ia").await.unwrap().unwrap().status, InboxStatus::Closed);
    assert!(!store.get_ticket_summary("a").await.unwrap().unwrap().proposed);
    assert!(ids(&store, in_project()).await.contains(&"a".to_string()));
}

#[tokio::test]
async fn ticket_unblocked_fires_when_the_last_blocker_closes_or_goes() {
    use pdt_core::workflow::events::TICKET_UNBLOCKED;
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    for id in ["a", "b1", "b2", "c"] {
        add(&store, id, new(id)).await;
    }
    store.add_link("b1", "a", LinkKind::Blocks, 2).await.unwrap();
    store.add_link("b2", "a", LinkKind::Blocks, 2).await.unwrap();
    let done = ValidPatch { stage: Some(TicketStage::Done), ..Default::default() };
    store.patch_ticket("b1", &done, 3).await.unwrap();
    assert!(logged(&store, TICKET_UNBLOCKED, "a").await.is_empty(), "b2 still blocks it");
    let wont = ValidPatch { stage: Some(TicketStage::WontDo), ..Default::default() };
    store.patch_ticket("b2", &wont, 4).await.unwrap();
    let [e] = logged(&store, TICKET_UNBLOCKED, "a").await.try_into().unwrap();
    assert_eq!((e["blockerId"].as_str(), e["reason"].as_str()), (Some("b2"), Some("closed")));
    assert!(e["blockerKey"].as_str().is_some_and(|k| k.ends_with("-3")), "{e}");
    // Reopening and closing a blocker of nothing else fires nothing more.
    store.patch_ticket("b2", &ValidPatch { stage: Some(TicketStage::Todo), ..Default::default() }, 5).await.unwrap();
    store.remove_link("b2", "a", LinkKind::Blocks, 6).await.unwrap();
    let unlinked = logged(&store, TICKET_UNBLOCKED, "a").await;
    assert_eq!(unlinked.len(), 2, "removing the last open blocker's link unblocks it");
    assert_eq!(unlinked[1]["reason"], "unlinked");
    // A link to a closed blocker going unblocks nothing.
    store.add_link("b1", "c", LinkKind::Blocks, 7).await.unwrap();
    store.remove_link("b1", "c", LinkKind::Blocks, 8).await.unwrap();
    assert!(logged(&store, TICKET_UNBLOCKED, "c").await.is_empty());
}

#[tokio::test]
async fn labels_priority_and_parent_changes_fire_once_each() {
    use pdt_core::workflow::events::{TICKET_LABELS_CHANGED, TICKET_PARENT_CHANGED, TICKET_PRIORITY_CHANGED};
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "t", ValidNewTicket { labels: vec!["a".into(), "b".into()], ..new("t") }).await;
    add(&store, "p1", new("p1")).await;
    add(&store, "p2", new("p2")).await;
    let p = ValidPatch {
        labels: Some(vec!["b".into(), "c".into()]),
        priority: Some(Priority::Urgent),
        parent: Some(Some("p1".into())),
        ..Default::default()
    };
    store.patch_ticket_by("t", &p, &coder_on("PX-9"), 2).await.unwrap();
    let [l] = logged(&store, TICKET_LABELS_CHANGED, "t").await.try_into().unwrap();
    assert_eq!(
        (l["added"].clone(), l["removed"].clone(), l["labels"].clone()),
        (json!(["c"]), json!(["a"]), json!(["b", "c"]))
    );
    assert_eq!(
        (l["actor"].as_str(), l["persona"].as_str(), l["sessionId"].as_str()),
        (Some("agent"), Some("Coder"), Some("s1"))
    );
    let [pr] = logged(&store, TICKET_PRIORITY_CHANGED, "t").await.try_into().unwrap();
    assert_eq!((pr["from"].as_str(), pr["to"].as_str()), (Some("none"), Some("urgent")));
    let [pa] = logged(&store, TICKET_PARENT_CHANGED, "t").await.try_into().unwrap();
    assert_eq!((pa["fromId"].clone(), pa["from"].as_str(), pa["toId"].as_str()), (json!(null), Some(""), Some("p1")));
    assert!(pa["to"].as_str().is_some_and(|k| k.ends_with("-2")), "{pa}");

    // The same values again change nothing; a move between parents is one change.
    store.patch_ticket("t", &p, 3).await.unwrap();
    store.patch_ticket("t", &ValidPatch { parent: Some(Some("p2".into())), ..Default::default() }, 4).await.unwrap();
    assert_eq!(logged(&store, TICKET_LABELS_CHANGED, "t").await.len(), 1);
    assert_eq!(logged(&store, TICKET_PRIORITY_CHANGED, "t").await.len(), 1);
    let moved = logged(&store, TICKET_PARENT_CHANGED, "t").await;
    assert_eq!(moved.len(), 2);
    assert_eq!(
        (moved[1]["fromId"].as_str(), moved[1]["toId"].as_str(), moved[1]["actor"].as_str()),
        (Some("p1"), Some("p2"), Some("user"))
    );
    store.patch_ticket("t", &ValidPatch { parent: Some(None), ..Default::default() }, 5).await.unwrap();
    assert_eq!(logged(&store, TICKET_PARENT_CHANGED, "t").await[2]["to"], "", "detached");

    // Bulk actions too, only for the tickets they change.
    store.bulk_tickets(&["t".into(), "p1".into()], &ValidBulk::AddLabel("c".into()), 6).await.unwrap();
    assert_eq!(logged(&store, TICKET_LABELS_CHANGED, "t").await.len(), 1, "t has c already");
    let [l] = logged(&store, TICKET_LABELS_CHANGED, "p1").await.try_into().unwrap();
    assert_eq!((l["added"].clone(), l["removed"].clone()), (json!(["c"]), json!([])));
    store.bulk_tickets(&["t".into()], &ValidBulk::RemoveLabel("b".into()), 7).await.unwrap();
    assert_eq!(logged(&store, TICKET_LABELS_CHANGED, "t").await[1]["removed"], json!(["b"]));
    store.bulk_tickets(&["t".into(), "p1".into()], &ValidBulk::Priority(Priority::Urgent), 8).await.unwrap();
    assert_eq!(logged(&store, TICKET_PRIORITY_CHANGED, "t").await.len(), 1, "t is urgent already");
    let [pr] = logged(&store, TICKET_PRIORITY_CHANGED, "p1").await.try_into().unwrap();
    assert_eq!((pr["from"].as_str(), pr["to"].as_str()), (Some("none"), Some("urgent")));
}

/// PDT-363: tickets agents made before keep who made them, and a non-PM agent's are `agent`.
#[tokio::test]
async fn the_agent_source_migration_fills_in_who_made_each_ticket() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("m");
    std::fs::create_dir_all(&old).unwrap();
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for e in std::fs::read_dir(&src).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with("_ticket_source_agent.sql") {
            std::fs::copy(e.path(), old.join(&name)).unwrap();
        }
    }
    let (db, backups) = (tmp.path().join("pdt.db"), tmp.path().join("backups"));
    let migrator = sqlx::migrate::Migrator::new(old.as_path()).await.unwrap();
    let store = Store::open_with(&db, &migrator, &backups).await.unwrap();
    let p = Project {
        id: "p".into(),
        name: "P".into(),
        repo_path: "/r".into(),
        base_branch: "main".into(),
        created_at: 1,
        key: String::new(),
        forge: None,
        kind: Default::default(),
    };
    store.insert_project(&p).await.unwrap();
    let agent = json!({"agent": {"sessionId": "s1", "persona": "Coder", "fromTicket": "PX-9"}});
    for (id, n, source, data) in
        [("a", 1, "workflow", Some(agent.clone())), ("pm", 2, "pm", Some(agent)), ("u", 3, "user", None)]
    {
        sqlx::query(
            "INSERT INTO tickets (id, seq, project_id, number, title, description, stage, priority, rank, \
             created_at, updated_at, source) VALUES (?, ?, 'p', ?, ?, '', 'draft', 'none', ?, 1, 1, ?)",
        )
        .bind(id)
        .bind(n)
        .bind(n)
        .bind(id)
        .bind(format!("a{n}"))
        .bind(source)
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ticket_events (id, ticket_id, kind, actor, data, created_at) VALUES (?, ?, 'created', ?, ?, 1)")
            .bind(format!("e{n}"))
            .bind(id)
            .bind(if data.is_some() { "agent" } else { "user" })
            .bind(data.map(|d| d.to_string()))
            .execute(&store.pool)
            .await
            .unwrap();
    }
    store.close().await;

    let store = Store::open_with(&db, &crate::migrate::MIGRATOR, &backups).await.unwrap();
    let a = store.get_ticket_summary("a").await.unwrap().unwrap();
    assert_eq!(a.source, TicketSource::Agent);
    let by = a.created_by.unwrap();
    assert_eq!(
        (by.session_id.as_str(), by.persona.as_deref(), by.from_ticket.as_deref()),
        ("s1", Some("Coder"), Some("PX-9"))
    );
    let pm = store.get_ticket_summary("pm").await.unwrap().unwrap();
    assert_eq!((pm.source, pm.created_by.is_some()), (TicketSource::Pm, true), "the PM's stay the PM's");
    let u = store.get_ticket_summary("u").await.unwrap().unwrap();
    assert_eq!((u.source, u.created_by), (TicketSource::User, None));
}
