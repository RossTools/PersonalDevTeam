use super::*;

fn agent_session(id: &str, attempt: Option<&str>, created_at: i64) -> pdt_core::session::Session {
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
    Session {
        id: id.into(),
        project_id: "p".into(),
        worktree_id: None,
        kind: SessionKind::AgentTui,
        agent: Some(AgentKind::Claude),
        bypass_permissions: false,
        title: format!("session {id}"),
        cwd: "/r".into(),
        status: SessionStatus::Running,
        exit_code: None,
        created_at,
        ended_at: None,
        external_session_id: None,
        fresh_started_at: None,
        activity: None,
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
        attempt_id: attempt.map(Into::into),
        ticket: None,
        mode: pdt_core::conversation::SessionMode::Terminal,
        model: None,
        permission_mode: None,
        persona: None,
        background: vec![],
    }
}

#[tokio::test]
async fn starting_an_attempt_moves_the_ticket_and_can_be_undone() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let t = add(&store, "a", ValidNewTicket { stage: TicketStage::Ready, ..new("Fix the login") }).await;
    assert_eq!(t.default_branch, "pdt-PX-1-fix-the-login");
    let before = store.get_ticket("a").await.unwrap().unwrap();

    let started = store
        .start_attempt("at1", "a", None, "pdt-PX-1-fix-the-login", "main", "abc", json!({"forced": true}), 5)
        .await
        .unwrap();
    assert_eq!((started.attempt.state, started.attempt.last_head.as_deref()), (AttemptState::Active, None));
    let t = store.get_ticket("a").await.unwrap().unwrap();
    assert_eq!(t.summary.stage, TicketStage::InProgress);
    assert_eq!(t.attempts.len(), 1);
    assert_eq!(events(&store, "a").await, [EventKind::Created, EventKind::Moved, EventKind::Started]);
    let page = store.list_events("a", None, 10, true).await.unwrap().unwrap();
    let data = page.items[0].data.clone().unwrap();
    assert_eq!((data["attemptId"].as_str(), data["forced"].as_bool()), (Some("at1"), Some(true)));
    // Workflows see none of it until Start announces it (its session exists).
    let log = |store: Store| async move {
        let f = crate::EventFilter { ticket_id: Some("a".into()), limit: 50, ..Default::default() };
        store.list_log_events(&f).await.unwrap().into_iter().map(|e| e.kind).collect::<Vec<_>>()
    };
    assert_eq!(log(store.clone()).await, ["ticket.created"], "only its making");

    // One active attempt per ticket.
    let again = store.start_attempt("at2", "a", None, "b", "main", "abc", json!({}), 6).await;
    assert!(matches!(again, Err(TicketError::Active(k)) if k == "PX-1"));
    assert!(matches!(
        store.start_attempt("at2", "nope", None, "b", "main", "abc", json!({}), 6).await,
        Err(TicketError::NotFound("ticket"))
    ));

    // Undo puts everything back, the pipeline stage too.
    store.undo_start(&started).await.unwrap();
    let t = store.get_ticket("a").await.unwrap().unwrap();
    assert_eq!((t.summary.stage, t.summary.updated_at), (TicketStage::Ready, before.summary.updated_at));
    assert!(t.attempts.is_empty());
    assert_eq!(events(&store, "a").await, [EventKind::Created]);
    assert_eq!(log(store.clone()).await, ["ticket.created"], "no workflow ever saw the undone start");
    store.set_pipeline_stage("a", Some(PipelineStage::Code)).await.unwrap();
    let started = store.start_attempt("at3", "a", None, "b", "main", "abc", json!({}), 7).await.unwrap();
    assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().pipeline_stage, None);
    store.undo_start(&started).await.unwrap();
    assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().pipeline_stage, Some(PipelineStage::Code));

    // Announced, the start is in the log.
    let started = store.start_attempt("at4", "a", None, "b", "main", "abc", json!({}), 8).await.unwrap();
    store.announce_start(&started, 9).await.unwrap();
    assert_eq!(log(store.clone()).await, ["ticket.created", "ticket.stage_changed", "attempt.started"]);

    // A ticket already in progress gets no `moved` event.
    add(&store, "b", ValidNewTicket { stage: TicketStage::InProgress, ..new("B") }).await;
    store.start_attempt("bt1", "b", None, "x", "main", "abc", json!({}), 7).await.unwrap();
    assert_eq!(events(&store, "b").await, [EventKind::Created, EventKind::Started]);
}

#[tokio::test]
async fn attempts_end_once_and_remember_their_head() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", ValidNewTicket { labels: vec!["automerge".into(), "docs".into()], ..new("A") }).await;
    let started = store.start_attempt("at1", "a", None, "br", "main", "abc", json!({}), 5).await.unwrap();
    store.announce_start(&started, 5).await.unwrap();
    let active = store.list_active_attempts().await.unwrap();
    assert_eq!(active.len(), 1);
    let info = &active[0];
    assert_eq!((info.ticket_key.as_str(), info.repo_path.as_str(), info.base_commit.as_str()), ("PX-1", "/r", "abc"));
    assert_eq!(info.worktree_path, None);
    assert_eq!(store.active_attempt("a").await.unwrap().map(|a| a.attempt.id), Some("at1".into()));

    assert!(!info.own_commits);
    assert!(store.set_attempt_head("at1", "def", false).await.unwrap());
    let info = store.get_attempt("at1").await.unwrap().unwrap();
    assert_eq!((info.attempt.last_head.as_deref(), info.own_commits), (Some("def"), false));
    assert!(store.set_attempt_head("at1", "ghi", true).await.unwrap());
    assert!(store.set_attempt_head("at1", "def", false).await.unwrap());
    let info = store.get_attempt("at1").await.unwrap().unwrap();
    assert_eq!((info.attempt.last_head.as_deref(), info.own_commits), (Some("def"), true), "own commits stick");

    // Finish on merge (Merge… on a pull request); deleting the branch only comes with finishing.
    assert_eq!((info.finish_on_merge, info.finish_delete_branch), (false, false));
    assert!(store.set_attempt_finish("at1", true, true).await.unwrap());
    let info = store.get_attempt("at1").await.unwrap().unwrap();
    assert_eq!((info.finish_on_merge, info.finish_delete_branch), (true, true));
    assert!(store.set_attempt_finish("at1", false, true).await.unwrap());
    let info = store.get_attempt("at1").await.unwrap().unwrap();
    assert_eq!((info.finish_on_merge, info.finish_delete_branch), (false, false));
    assert!(!store.set_attempt_finish("nope", true, false).await.unwrap());

    // Set to merge (PDT-104): sticky, news once.
    assert!(!info.merge_requested);
    assert!(store.set_attempt_merge_requested("at1").await.unwrap());
    assert!(!store.set_attempt_merge_requested("at1").await.unwrap());
    assert!(store.get_attempt("at1").await.unwrap().unwrap().merge_requested);
    assert!(store.clear_attempt_merge_requested("at1").await.unwrap());
    assert!(!store.get_attempt("at1").await.unwrap().unwrap().merge_requested);
    assert!(store.set_attempt_merge_refused("at1").await.unwrap());
    assert!(!store.set_attempt_merge_refused("at1").await.unwrap());
    assert!(store.get_attempt("at1").await.unwrap().unwrap().merge_refused);
    // Set to merge takes over from a refused merge; turned off by hand forgets both (PDT-139).
    assert!(store.set_attempt_merge_requested("at1").await.unwrap());
    assert!(!store.get_attempt("at1").await.unwrap().unwrap().merge_refused);
    assert!(!store.set_attempt_merge_refused("at1").await.unwrap(), "not while set to merge");
    assert!(store.clear_attempt_merge_requested("at1").await.unwrap());
    assert!(store.set_attempt_merge_refused("at1").await.unwrap());
    assert!(store.clear_attempt_merge_requested("at1").await.unwrap());
    let info = store.get_attempt("at1").await.unwrap().unwrap();
    assert!(!info.merge_requested && !info.merge_refused);

    // The workspace Start made is remembered, and forgotten when the workspace is deleted.
    assert_eq!(store.get_attempt("at1").await.unwrap().unwrap().attempt.workspace_id, None);
    let w = pdt_core::workspace::Workspace {
        id: "ws".into(),
        name: "PX-1".into(),
        project_id: None,
        position: 0,
        created_at: 5,
        tabs: vec![],
        ticket: None,
    };
    store.insert_workspace(&w).await.unwrap();
    assert!(store.set_attempt_workspace("at1", "ws").await.unwrap());
    assert_eq!(store.get_attempt("at1").await.unwrap().unwrap().attempt.workspace_id.as_deref(), Some("ws"));
    assert_eq!(store.get_ticket("a").await.unwrap().unwrap().attempts[0].workspace_id.as_deref(), Some("ws"));
    store.delete_workspace("ws", false).await.unwrap();
    assert_eq!(store.get_attempt("at1").await.unwrap().unwrap().attempt.workspace_id, None);

    assert_eq!(
        store.end_attempt("at1", AttemptState::Merged, Actor::System, None, Some(4), 9).await.unwrap().as_deref(),
        Some("a")
    );
    assert_eq!(store.end_attempt("at1", AttemptState::Abandoned, Actor::User, None, None, 10).await.unwrap(), None);
    let a = store.get_attempt("at1").await.unwrap().unwrap().attempt;
    assert_eq!((a.state, a.ended_at), (AttemptState::Merged, Some(9)));
    assert!(store.list_active_attempts().await.unwrap().is_empty());
    assert!(store.active_attempt("a").await.unwrap().is_none());
    assert_eq!(
        events(&store, "a").await,
        [EventKind::Created, EventKind::Moved, EventKind::Started, EventKind::Merged]
    );
    // The domain events are mirrored into the event log with the change.
    let log = store
        .list_log_events(&crate::EventFilter { ticket_id: Some("a".into()), limit: 50, ..Default::default() })
        .await
        .unwrap();
    let kinds: Vec<&str> = log.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["ticket.created", "ticket.stage_changed", "attempt.started", "attempt.merged"]);
    assert_eq!(log[2].payload["labels"], json!(["automerge", "docs"]), "a condition can branch on them");
    let merged = &log[3].payload;
    assert_eq!(
        (merged["via"].as_str(), merged["pr"].as_u64(), merged["attemptId"].as_str()),
        (Some("detected"), Some(4), Some("at1"))
    );
    assert!(merged["notice"]["text"].as_str().unwrap().contains("PR #4"));
    assert_eq!(merged["ticketTitle"], "A", "a sheet's log row can name it");
    assert_eq!(log[3].key, "at1:merged");
    store.add_comment("a", "please also fix x", 10).await.unwrap();
    let log = store
        .list_log_events(&crate::EventFilter { kind: Some("ticket.commented".into()), limit: 5, ..Default::default() })
        .await
        .unwrap();
    assert_eq!(
        (log[0].payload["body"].as_str(), log[0].payload["attemptId"].as_str()),
        (Some("please also fix x"), None)
    );
    // A new attempt can start after the last one ended.
    store.start_attempt("at2", "a", None, "br", "main", "abc", json!({}), 11).await.unwrap();
    assert_eq!(
        store.end_attempt("at2", AttemptState::Abandoned, Actor::User, None, None, 12).await.unwrap().as_deref(),
        Some("a")
    );
    assert_eq!(events(&store, "a").await.last(), Some(&EventKind::Abandoned));
}

#[tokio::test]
async fn sessions_carry_their_ticket_and_tickets_list_their_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", new("A")).await;
    store.start_attempt("at1", "a", None, "br", "main", "abc", json!({}), 5).await.unwrap();
    store.insert_session(&agent_session("s1", Some("at1"), 1)).await.unwrap();
    store.insert_session(&agent_session("s2", Some("at1"), 2)).await.unwrap();
    store.insert_session(&agent_session("plain", None, 3)).await.unwrap();

    let s = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!(s.attempt_id.as_deref(), Some("at1"));
    let t = s.ticket.unwrap();
    assert_eq!((t.id.as_str(), t.key.as_str()), ("a", "PX-1"));
    let plain = store.get_session("plain").await.unwrap().unwrap();
    assert_eq!((plain.attempt_id, plain.ticket), (None, None));
    let listed = store.list_sessions("p").await.unwrap();
    assert_eq!(listed.iter().filter(|s| s.ticket.is_some()).count(), 2);

    let ticket = store.get_ticket("a").await.unwrap().unwrap();
    let ids: Vec<&str> = ticket.sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["s2", "s1"], "newest first");
    assert_eq!(ticket.sessions[0].attempt_id.as_deref(), Some("at1"));

    // A workflow step's session on the ticket that isn't the attempt's (a Reviewer) belongs to the ticket too.
    let mut reviewer = agent_session("rev", None, 4);
    reviewer.ticket = Some(pdt_core::ticket::SessionTicket { id: "a".into(), key: String::new() });
    store.insert_session(&reviewer).await.unwrap();
    let s = store.get_session("rev").await.unwrap().unwrap();
    assert_eq!((s.attempt_id, s.ticket.map(|t| t.key)), (None, Some("PX-1".into())));
    let ticket = store.get_ticket("a").await.unwrap().unwrap();
    let ids: Vec<(&str, Option<&str>)> =
        ticket.sessions.iter().map(|s| (s.id.as_str(), s.attempt_id.as_deref())).collect();
    assert_eq!(ids, [("rev", None), ("s2", Some("at1")), ("s1", Some("at1"))]);
    store.delete_session("rev").await.unwrap();

    // A chat joins an attempt once; a session with one keeps it.
    assert!(store.set_session_attempt("plain", "at1").await.unwrap());
    assert!(!store.set_session_attempt("plain", "at1").await.unwrap());
    assert!(!store.set_session_attempt("s1", "at1").await.unwrap());
    assert!(!store.set_session_attempt("gone", "at1").await.unwrap());
    let ticket = store.get_ticket("a").await.unwrap().unwrap();
    assert_eq!(ticket.sessions.len(), 3);
    store.delete_session("plain").await.unwrap();

    // Deleting the ticket (here directly: the API refuses while the attempt is active) drops the attempt; its
    // sessions lose the link.
    assert_eq!(store.delete_ticket("a").await.unwrap(), DeleteTicket::Active);
    sqlx::query("DELETE FROM tickets WHERE id = 'a'").execute(&store.pool).await.unwrap();
    let s = store.get_session("s1").await.unwrap().unwrap();
    assert_eq!((s.attempt_id, s.ticket), (None, None));
}
