use super::*;

#[tokio::test]
async fn external_tickets_reach_the_board_only_through_the_user() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let ext = |stage| ValidNewTicket { source: TicketSource::External, stage, ..new("From an issue") };
    let a = add(&store, "a", ext(TicketStage::Draft)).await;
    assert_eq!((a.summary.source, a.summary.admitted), (TicketSource::External, false));
    // Mine default to the user's.
    assert_eq!(add(&store, "u", new("Mine")).await.summary.source, TicketSource::User);
    // A workflow or an agent can't make one on the board, move it there or label it agentic.
    let made = store.create_ticket_by("b", "p", &ext(TicketStage::Todo), Actor::System, 2).await;
    assert!(matches!(made, Err(TicketError::Untrusted(_))), "{made:?}");
    let moved = store.move_ticket_by("a", TicketStage::Todo, Actor::System, None, 2).await;
    assert!(matches!(moved, Err(TicketError::Untrusted(_))), "{moved:?}");
    let agent = By::agent(AgentBy {
        session_id: "s".into(),
        project_id: None,
        persona: Some("Refiner".into()),
        from_ticket: None,
        from_ticket_id: None,
    });
    let start = ValidPatch { stage: Some(TicketStage::InProgress), ..Default::default() };
    assert!(matches!(store.patch_ticket_by("a", &start, &agent, 2).await, Err(TicketError::Untrusted(_))));
    let label = ValidPatch { labels: Some(vec![pdt_core::ticket::AGENTIC_LABEL.into()]), ..Default::default() };
    assert!(matches!(store.patch_ticket_by("a", &label, &agent, 2).await, Err(TicketError::Untrusted(_))));
    let a = store.get_ticket("a").await.unwrap().unwrap();
    assert_eq!((a.summary.stage, a.summary.labels.len()), (TicketStage::Draft, 0));
    // Ready is fine.
    store.move_ticket_by("a", TicketStage::Ready, Actor::System, None, 3).await.unwrap();
    // A call the user confirmed lets it in.
    let confirmed = By { confirmed: true, ..agent.clone() };
    let t = store.patch_ticket_by("a", &label, &confirmed, 4).await.unwrap().unwrap();
    assert!(t.summary.admitted);
    // Once let in, workflows move it as any other.
    store.move_ticket_by("a", TicketStage::Todo, Actor::System, None, 5).await.unwrap();
    // The user moving one to To do lets it in.
    add(&store, "c", ext(TicketStage::Draft)).await;
    let todo = ValidPatch { stage: Some(TicketStage::Todo), ..Default::default() };
    assert!(store.patch_ticket("c", &todo, 6).await.unwrap().unwrap().summary.admitted);
    // As do the bulk move and starting it.
    add(&store, "d", ext(TicketStage::Ready)).await;
    store.bulk_tickets(&["d".into()], &ValidBulk::Stage(TicketStage::Todo), 7).await.unwrap();
    assert!(store.get_ticket_summary("d").await.unwrap().unwrap().admitted);
    add(&store, "e", ext(TicketStage::Draft)).await;
    store.start_attempt("at-e", "e", None, "b", "main", "", json!({}), 8).await.unwrap();
    assert!(store.get_ticket_summary("e").await.unwrap().unwrap().admitted);
    // The user making one in To do lets it in at once.
    assert!(add(&store, "f", ext(TicketStage::Todo)).await.summary.admitted);
}

#[tokio::test]
async fn a_triaged_tickets_inbox_item_closes_once_it_moves_past_ready() {
    use pdt_core::inbox::{InboxGroup, InboxItem, InboxKind, InboxStatus};
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let admit = |id: &str, ticket: &str| InboxItem {
        id: id.into(),
        kind: InboxKind::Admit,
        group: InboxGroup::Admit,
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
        title: "Approve?".into(),
        body: json!({}),
        choices: vec![],
        resolution: serde_json::Value::Null,
        created_at: 1,
        resolved_at: None,
    };
    let open = |store: &Store| {
        let store = store.clone();
        async move {
            let items = store.open_inbox_items_of(InboxKind::Admit).await.unwrap();
            items.into_iter().map(|i| i.id).collect::<Vec<_>>()
        }
    };
    add(&store, "a", new("Triaged")).await;
    add(&store, "b", new("Also triaged")).await;
    store.add_inbox_item_once(&admit("ia", "a"), "admit:a").await.unwrap();
    store.add_inbox_item_once(&admit("ib", "b"), "admit:b").await.unwrap();
    // Refining it to Ready leaves it waiting.
    store.move_ticket_by("a", TicketStage::Ready, Actor::System, None, 2).await.unwrap();
    assert_eq!(open(&store).await, ["ia", "ib"]);
    // Moved on the board, it closes; deleted, it goes.
    let todo = ValidPatch { stage: Some(TicketStage::Todo), ..Default::default() };
    store.patch_ticket("a", &todo, 3).await.unwrap();
    assert_eq!(open(&store).await, ["ib"]);
    let a = store.get_inbox_item("ia").await.unwrap().unwrap();
    assert_eq!((a.status, a.resolution), (InboxStatus::Closed, json!({"moved": "todo"})));
    assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::Deleted);
    assert!(open(&store).await.is_empty());
}

#[tokio::test]
async fn edits_record_events_and_closing_sets_closed_at() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", new("A")).await;
    let p = ValidPatch {
        title: Some("A2".into()),
        labels: Some(vec!["x".into()]),
        stage: Some(TicketStage::Done),
        ..Default::default()
    };
    let t = store.patch_ticket("a", &p, 2000).await.unwrap().unwrap();
    assert_eq!((t.summary.title.as_str(), t.summary.stage, t.summary.closed_at), ("A2", TicketStage::Done, Some(2000)));
    assert_eq!(t.summary.updated_at, 2000);
    let p = ValidPatch { stage: Some(TicketStage::WontDo), ..Default::default() };
    let t = store.patch_ticket("a", &p, 3000).await.unwrap().unwrap();
    assert_eq!(t.summary.closed_at, Some(2000), "still closed since 2000");
    let p = ValidPatch { stage: Some(TicketStage::Ready), title: Some("A2".into()), ..Default::default() };
    let t = store.patch_ticket("a", &p, 4000).await.unwrap().unwrap();
    assert_eq!(t.summary.closed_at, None);
    assert_eq!(
        events(&store, "a").await,
        [EventKind::Created, EventKind::Edited, EventKind::Moved, EventKind::Moved, EventKind::Moved]
    );
    let page = store.list_events("a", None, 100, false).await.unwrap().unwrap();
    assert_eq!(page.items[1].data, Some(json!({"fields": ["title", "labels"]})));
    assert_eq!(page.items[2].data, Some(json!({"from": "draft", "to": "done"})));
    assert!(store.patch_ticket("nope", &ValidPatch::default(), 1).await.unwrap().is_none());
}

#[tokio::test]
async fn bulk_actions_are_all_or_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    for id in ["c", "b", "a"] {
        add(&store, id, new(id)).await;
    }
    let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let r = store.bulk_tickets(&v(&["a", "nope"]), &ValidBulk::Stage(TicketStage::Ready), 5).await;
    assert!(matches!(r, Err(TicketError::NotFound("ticket"))));
    assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().stage, TicketStage::Draft);

    assert_eq!(store.bulk_tickets(&v(&["a", "b", "a"]), &ValidBulk::Stage(TicketStage::Ready), 5).await.unwrap(), 2);
    assert_eq!(store.bulk_tickets(&v(&["a", "c"]), &ValidBulk::Stage(TicketStage::Ready), 5).await.unwrap(), 1);
    assert_eq!(store.bulk_tickets(&v(&["a"]), &ValidBulk::Priority(Priority::Urgent), 5).await.unwrap(), 1);
    assert_eq!(store.bulk_tickets(&v(&["a", "b"]), &ValidBulk::AddLabel("ui".into()), 5).await.unwrap(), 2);
    assert_eq!(store.bulk_tickets(&v(&["a", "b"]), &ValidBulk::RemoveLabel("ui".into()), 5).await.unwrap(), 2);
    assert_eq!(store.bulk_tickets(&v(&["c"]), &ValidBulk::RemoveLabel("ui".into()), 5).await.unwrap(), 0);

    // One ticket over the label limit undoes the whole action.
    let full: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
    store.patch_ticket("b", &ValidPatch { labels: Some(full), ..Default::default() }, 6).await.unwrap();
    let r = store.bulk_tickets(&v(&["a", "b"]), &ValidBulk::AddLabel("new".into()), 7).await;
    assert!(matches!(r, Err(TicketError::Invalid(_))));
    assert!(store.get_ticket_summary("a").await.unwrap().unwrap().labels.is_empty());

    assert_eq!(ids(&store, in_project()).await, ["a", "b", "c"]);
    store.bulk_tickets(&v(&["c", "b"]), &ValidBulk::Top, 8).await.unwrap();
    assert_eq!(ids(&store, in_project()).await, ["b", "c", "a"]);

    // An active attempt blocks a bulk delete of any of them.
    sqlx::query(
        "INSERT INTO attempts (id, ticket_id, branch, base_branch, base_commit, state, created_at) \
         VALUES ('at', 'a', 'pdt/x', 'main', 'abc', 'active', 1)",
    )
    .execute(&store.pool)
    .await
    .unwrap();
    let r = store.bulk_tickets(&v(&["b", "a"]), &ValidBulk::Delete, 9).await;
    assert!(matches!(r, Err(TicketError::Active(k)) if k == "PX-3"));
    assert_eq!(store.delete_ticket("a").await.unwrap(), DeleteTicket::Active);
    assert_eq!(ids(&store, in_project()).await.len(), 3);
    assert_eq!(store.bulk_tickets(&v(&["b", "c"]), &ValidBulk::Delete, 9).await.unwrap(), 2);
    assert_eq!(ids(&store, in_project()).await, ["a"]);
    let summary = store.get_ticket_summary("a").await.unwrap().unwrap();
    assert_eq!(summary.attempt.map(|a| a.state), Some(AttemptState::Active));

    // One active attempt per ticket.
    let dup = sqlx::query(
        "INSERT INTO attempts (id, ticket_id, branch, base_branch, base_commit, state, created_at) \
         VALUES ('at2', 'a', 'pdt/y', 'main', 'abc', 'active', 2)",
    )
    .execute(&store.pool)
    .await;
    assert!(dup.is_err());
}

#[tokio::test]
async fn tickets_open_in_tabs_and_take_their_tab_with_them() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let home = store.project_workspace_id("p").await.unwrap().unwrap();
    for id in ["a", "b"] {
        add(&store, id, new(id)).await;
        store.open_tab(&format!("t{id}"), &home, &TabItem::Ticket(id.into()), None, 1).await.unwrap().unwrap();
    }
    let tab = store.tab_for(&TabItem::Ticket("b".into())).await.unwrap().unwrap();
    assert_eq!((tab.id.as_str(), tab.ticket_id.as_deref(), tab.position), ("tb", Some("b"), 1));
    // One tab per ticket: opening again moves it.
    store.open_tab("ignored", &home, &TabItem::Ticket("b".into()), Some(0), 1).await.unwrap();
    assert_eq!(store.get_workspace(&home).await.unwrap().unwrap().tabs[0].id, "tb");

    // A tab shows exactly one thing.
    let bad = sqlx::query(
        "INSERT INTO workspace_tabs (id, workspace_id, session_id, run_id, ticket_id, position, created_at) \
         VALUES ('x', ?, NULL, NULL, NULL, 9, 1)",
    )
    .bind(&home)
    .execute(&store.pool)
    .await;
    assert!(bad.is_err());

    // A free-form workspace's ticket tabs go home when it is deleted.
    let free = pdt_core::workspace::Workspace {
        id: "f".into(),
        name: "F".into(),
        project_id: None,
        position: 0,
        created_at: 1,
        tabs: vec![],
        ticket: None,
    };
    store.insert_workspace(&free).await.unwrap();
    store.move_tab("ta", Some("f"), None).await.unwrap().unwrap();
    store.delete_workspace("f", false).await.unwrap();
    assert_eq!(store.get_workspace(&home).await.unwrap().unwrap().tabs.len(), 2);

    assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::Deleted);
    let tabs = store.get_workspace(&home).await.unwrap().unwrap().tabs;
    assert_eq!(tabs.iter().map(|t| (t.id.as_str(), t.position)).collect::<Vec<_>>(), [("ta", 0)]);
}

fn patch() -> ValidPatch {
    ValidPatch::default()
}

#[tokio::test]
async fn waiting_goes_to_the_top_and_ends_when_started_or_closed() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", ValidNewTicket { stage: TicketStage::Ready, ..new("A") }).await;
    add(&store, "b", ValidNewTicket { stage: TicketStage::Ready, ..new("B") }).await;
    assert_eq!(ids(&store, in_project()).await, ["b", "a"]);

    let t = store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 5).await.unwrap().unwrap();
    assert!(t.summary.waiting);
    assert_eq!(ids(&store, in_project()).await, ["a", "b"], "a waiting ticket sits at the top of its stage");
    assert_eq!(events(&store, "a").await.last(), Some(&EventKind::Waiting));
    // Setting it again changes nothing.
    store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 6).await.unwrap();
    assert_eq!(events(&store, "a").await.iter().filter(|e| **e == EventKind::Waiting).count(), 1);
    let t = store.patch_ticket("a", &ValidPatch { waiting: Some(false), ..patch() }, 7).await.unwrap().unwrap();
    assert!(!t.summary.waiting);

    store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 8).await.unwrap();
    store.start_attempt("at1", "a", None, "x", "main", "abc", json!({}), 9).await.unwrap();
    assert!(!store.get_ticket_summary("a").await.unwrap().unwrap().waiting, "started");
    let r = store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 10).await;
    assert!(matches!(r, Err(TicketError::Invalid(_))), "an active ticket can't wait");

    store.patch_ticket("b", &ValidPatch { waiting: Some(true), ..patch() }, 11).await.unwrap();
    let closed = ValidPatch { stage: Some(TicketStage::WontDo), ..patch() };
    let t = store.patch_ticket("b", &closed, 12).await.unwrap().unwrap();
    assert!(!t.summary.waiting, "closed");
    let r = store.patch_ticket("b", &ValidPatch { waiting: Some(true), ..patch() }, 13).await;
    assert!(matches!(r, Err(TicketError::Invalid(_))), "a closed ticket can't wait");
}
