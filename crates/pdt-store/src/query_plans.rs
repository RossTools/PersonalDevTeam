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
            "DELETE FROM workflow_runs WHERE ended_at IS NOT NULL AND ended_at < ? \
             AND +status IN ('succeeded', 'quiet', 'failed', 'cancelled')",
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
