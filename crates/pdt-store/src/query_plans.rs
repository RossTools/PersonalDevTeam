//! The plans SQLite picks for the queries pdtd's loops run every few seconds (PDT-614): each must search an index,
//! never scan a table that grows with history (the event log, timelines, workflow runs and steps).

use crate::Store;

/// The `EXPLAIN QUERY PLAN` lines for `sql`, its `?`s bound to NULL.
pub(crate) async fn plan(store: &Store, sql: &str) -> Vec<String> {
    let mut q = sqlx::query_as::<_, (i64, i64, i64, String)>(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")));
    for _ in 0..sql.matches('?').count() {
        q = q.bind(None::<i64>);
    }
    q.fetch_all(&store.pool).await.unwrap().into_iter().map(|r| r.3).collect()
}

/// The lines that read a whole table or index (a partial index over open rows is fine: it stays small).
pub(crate) fn scans(plan: &[String]) -> Vec<&str> {
    plan.iter().map(String::as_str).filter(|d| d.starts_with("SCAN ")).collect()
}

#[tokio::test]
async fn the_engines_periodic_queries_search_indexes() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    // The WHERE clauses of `runnable_workflow_runs`, `wake_untimed_workflow_runs`, `steps_waiting_for_*`,
    // `prune_workflow_runs` and `active_agent_steps`.
    let cases: &[(&str, &[&str])] = &[
        (
            "SELECT r.id FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id \
             WHERE r.status = 'running' OR (r.status = 'waiting' AND r.wake_at IS NOT NULL AND r.wake_at <= ?) \
             ORDER BY r.started_at, r.id",
            &[],
        ),
        ("UPDATE workflow_runs SET wake_at = ? WHERE status = 'waiting' AND wake_at IS NULL", &[]),
        (
            "SELECT r.ticket_id FROM workflow_steps s JOIN workflow_runs r ON r.id = s.run_id \
             WHERE s.status = 'waiting' AND json_extract(s.wait, '$.slot') IS NOT NULL ORDER BY s.started_at, s.id",
            &[],
        ),
        (
            "DELETE FROM workflow_runs WHERE ended_at IS NOT NULL AND ended_at < MAX(?, ?) \
             AND +status IN ('succeeded', 'quiet', 'failed', 'cancelled') \
             AND (ended_at < ? OR (status = 'quiet' AND ended_at < ? AND trigger_event_id IS NOT NULL \
                  AND run_key = trigger_node || ':' || trigger_event_id))",
            &[],
        ),
        (
            "SELECT step_id FROM agent_steps WHERE state != 'done' ORDER BY created_at",
            &["SCAN agent_steps USING INDEX agent_steps_open"],
        ),
        ("DELETE FROM events WHERE key = ? AND source = 'tickets'", &[]),
        (
            "SELECT e.ticket_id, e.data FROM ticket_events e JOIN tickets t ON t.id = e.ticket_id \
             WHERE e.kind = 'overlap' AND t.stage NOT IN ('done', 'wont_do') ORDER BY e.created_at, e.id",
            &[],
        ),
    ];
    for (sql, allowed) in cases {
        let plan = plan(&store, sql).await;
        assert_eq!(scans(&plan), *allowed, "{sql}\n{plan:#?}");
    }
}

#[tokio::test]
async fn the_polled_queries_search_indexes() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    // PDT-652: `get_ticket`'s sessions, `list_attempt_sessions`, `list_sessions_by_status`, `workspace_tickets`,
    // `attempt_for_worktree`, `events_with_runs_since`, `list_workflow_runs` unfiltered, `unread_updates`,
    // `prune_inbox_items`, `list_active_runs`, `read_notifications` for a session, and deleting an attempt (its
    // sessions' `attempt_id` is set to NULL).
    let session_joins = "FROM sessions s LEFT JOIN attempts a ON a.id = s.attempt_id \
                         LEFT JOIN tickets t ON t.id = COALESCE(a.ticket_id, s.ticket_id) \
                         LEFT JOIN projects p ON p.id = t.project_id LEFT JOIN personas pe ON pe.id = s.persona_id";
    let attempt_joins = "FROM attempts a JOIN tickets t ON t.id = a.ticket_id \
                         JOIN projects p ON p.id = t.project_id LEFT JOIN worktrees w ON w.id = a.worktree_id";
    let cases: Vec<(String, &[&str])> = vec![
        (
            "SELECT s.id, pe.name FROM sessions s LEFT JOIN personas pe ON pe.id = s.persona_id \
             WHERE s.attempt_id IN (SELECT id FROM attempts WHERE ticket_id = ?) \
             OR (s.ticket_id = ? AND s.attempt_id IS NULL) ORDER BY s.created_at DESC, s.id DESC"
                .into(),
            &[],
        ),
        (format!("SELECT s.id {session_joins} WHERE s.attempt_id = ? ORDER BY s.created_at, s.id"), &[]),
        (format!("SELECT s.id {session_joins} WHERE s.status = ? ORDER BY s.created_at, s.id"), &[]),
        ("UPDATE sessions SET attempt_id = NULL WHERE attempt_id = ?".into(), &[]),
        (
            "SELECT a.workspace_id, t.id FROM attempts a JOIN tickets t ON t.id = a.ticket_id \
             JOIN projects p ON p.id = t.project_id WHERE a.workspace_id IS NOT NULL ORDER BY a.created_at, a.id"
                .into(),
            &[],
        ),
        (
            format!(
                "SELECT a.id {attempt_joins} WHERE a.worktree_id = ? \
                 ORDER BY a.state = 'active' DESC, a.created_at DESC LIMIT 1"
            ),
            &[],
        ),
        (
            "SELECT COUNT(DISTINCT trigger_event_id) FROM workflow_runs WHERE started_at >= ? \
             AND trigger_event_id IS NOT NULL AND trigger_event_id != ? \
             AND json_extract(trigger, '$.type') = ? AND json_extract(trigger, '$.' || ?) = ?"
                .into(),
            &[],
        ),
        (
            "SELECT r.id FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id WHERE 1 = 1 \
             ORDER BY r.started_at DESC, r.id DESC LIMIT ?"
                .into(),
            &["SCAN r USING INDEX workflow_runs_by_started"],
        ),
        (
            "SELECT COUNT(*) FROM notifications WHERE needs_you = 0 AND read_at IS NULL AND cleared_at IS NULL \
             AND dismissed_at IS NULL AND (? IS NULL OR project_id = ? OR project_id IS NULL)"
                .into(),
            &[],
        ),
        ("DELETE FROM inbox_items WHERE status != 'open' AND resolved_at < ?".into(), &[]),
        (
            "SELECT id FROM runs WHERE state IN ('setting_up', 'starting', 'ready', 'running') ORDER BY created_at, id"
                .into(),
            &["SCAN runs USING INDEX runs_active"],
        ),
        ("UPDATE notifications SET read_at = ? WHERE read_at IS NULL AND session_id = ?".into(), &[]),
    ];
    for (sql, allowed) in &cases {
        let plan = plan(&store, sql).await;
        assert_eq!(scans(&plan), *allowed, "{sql}\n{plan:#?}");
    }
    // Searching `notifications_feed` on `needs_you = 0` would read most of the table.
    let unread = plan(&store, &cases[8].0).await;
    assert_eq!(unread, ["SEARCH notifications USING INDEX notifications_unread (needs_you=?)"], "{unread:#?}");
}
