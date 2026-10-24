//! The Inbox's stored items and agent steps' session bindings (Phase 7b spec §4.2, §6).

use crate::{Store, StoreError};
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::workflow::agent::{AgentStep, AgentStepState};
use serde_json::Value;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const ITEM_COLUMNS: &str = "id, kind, grp, status, project_id, ticket_id, attempt_id, session_id, run_id, step_id, \
                            persona_id, title, body, choices, resolution, created_at, resolved_at";
const STEP_COLUMNS: &str = "step_id, run_id, node_id, session_id, persona_id, persona_name, persona_version, \
                            project_id, ticket_id, attempt_id, outcomes, state, turns, nudges, max_turns, auto_nudges, \
                            outcome, summary, comments, by_user, created_at, updated_at, background_since, \
                            background, background_wait_ms";

fn corrupt(what: &str, v: &str) -> StoreError {
    StoreError::Corrupt(format!("{what} {v:?}"))
}

fn json(s: Option<String>) -> Value {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn to_item(r: &SqliteRow) -> Result<InboxItem, StoreError> {
    let kind: String = r.try_get("kind")?;
    let group: String = r.try_get("grp")?;
    let status: String = r.try_get("status")?;
    let choices: Vec<InboxChoice> =
        serde_json::from_value(json(r.try_get("choices")?)).map_err(|e| StoreError::Corrupt(e.to_string()))?;
    Ok(InboxItem {
        id: r.try_get("id")?,
        kind: InboxKind::parse(&kind).ok_or_else(|| corrupt("inbox kind", &kind))?,
        group: InboxGroup::parse(&group).ok_or_else(|| corrupt("inbox group", &group))?,
        status: InboxStatus::parse(&status).ok_or_else(|| corrupt("inbox status", &status))?,
        project_id: r.try_get("project_id")?,
        ticket_id: r.try_get("ticket_id")?,
        ticket_key: None,
        ticket_title: None,
        attempt_id: r.try_get("attempt_id")?,
        session_id: r.try_get("session_id")?,
        run_id: r.try_get("run_id")?,
        step_id: r.try_get("step_id")?,
        persona_id: r.try_get("persona_id")?,
        persona_name: None,
        title: r.try_get("title")?,
        body: json(r.try_get("body")?),
        choices,
        resolution: json(r.try_get("resolution")?),
        created_at: r.try_get("created_at")?,
        resolved_at: r.try_get("resolved_at")?,
    })
}

fn to_step(r: &SqliteRow) -> Result<AgentStep, StoreError> {
    let state: String = r.try_get("state")?;
    let n = |k: &str| -> Result<u32, StoreError> { Ok(u32::try_from(r.try_get::<i64, _>(k)?).unwrap_or(0)) };
    Ok(AgentStep {
        step_id: r.try_get("step_id")?,
        run_id: r.try_get("run_id")?,
        node_id: r.try_get("node_id")?,
        session_id: r.try_get("session_id")?,
        persona_id: r.try_get("persona_id")?,
        persona_name: r.try_get("persona_name")?,
        persona_version: n("persona_version")?,
        project_id: r.try_get("project_id")?,
        ticket_id: r.try_get("ticket_id")?,
        attempt_id: r.try_get("attempt_id")?,
        outcomes: serde_json::from_value(json(r.try_get("outcomes")?)).unwrap_or_default(),
        state: AgentStepState::parse(&state).ok_or_else(|| corrupt("agent step state", &state))?,
        turns: n("turns")?,
        nudges: n("nudges")?,
        max_turns: n("max_turns")?,
        auto_nudges: n("auto_nudges")?,
        outcome: r.try_get("outcome")?,
        summary: r.try_get("summary")?,
        comments: serde_json::from_value(json(r.try_get("comments")?)).unwrap_or_default(),
        by_user: r.try_get::<i64, _>("by_user")? != 0,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        background_since: r.try_get("background_since")?,
        background: serde_json::from_value(json(r.try_get("background")?)).unwrap_or_default(),
        background_wait_ms: u64::try_from(r.try_get::<i64, _>("background_wait_ms")?).unwrap_or(0),
    })
}

impl Store {
    // ---- inbox ----

    /// Adds an item; an open item of the same kind for the same step is returned instead (a step re-run after a
    /// restart asks once).
    pub async fn open_inbox_item(&self, i: &InboxItem) -> Result<InboxItem, StoreError> {
        if let Some(step) = &i.step_id {
            let sql =
                format!("SELECT {ITEM_COLUMNS} FROM inbox_items WHERE step_id = ? AND kind = ? AND status = 'open'");
            let found = sqlx::query(sqlx::AssertSqlSafe(sql))
                .bind(step)
                .bind(i.kind.as_str())
                .fetch_optional(&self.pool)
                .await?;
            if let Some(r) = found {
                return to_item(&r);
            }
        }
        sqlx::query(
            "INSERT INTO inbox_items (id, kind, grp, status, project_id, ticket_id, attempt_id, session_id, run_id, \
             step_id, persona_id, title, body, choices, resolution, created_at, resolved_at) \
             VALUES (?, ?, ?, 'open', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, NULL)",
        )
        .bind(&i.id)
        .bind(i.kind.as_str())
        .bind(i.group.as_str())
        .bind(&i.project_id)
        .bind(&i.ticket_id)
        .bind(&i.attempt_id)
        .bind(&i.session_id)
        .bind(&i.run_id)
        .bind(&i.step_id)
        .bind(&i.persona_id)
        .bind(&i.title)
        .bind(i.body.to_string())
        .bind(serde_json::to_string(&i.choices).unwrap_or_else(|_| "[]".into()))
        .bind(i.created_at)
        .execute(&self.pool)
        .await?;
        Ok(self.get_inbox_item(&i.id).await?.unwrap_or_else(|| i.clone()))
    }

    /// Adds an item that is made once per `dedupe_key` (an unheard notice, PDT-139): None when an item with the key
    /// was ever made (open, answered or closed; pruned ones are forgotten).
    pub async fn add_inbox_item_once(&self, i: &InboxItem, dedupe_key: &str) -> Result<Option<InboxItem>, StoreError> {
        let r = sqlx::query(
            "INSERT INTO inbox_items (id, kind, grp, status, project_id, ticket_id, attempt_id, session_id, run_id, \
             step_id, persona_id, title, body, choices, resolution, created_at, resolved_at, dedupe_key) \
             SELECT ?, ?, ?, 'open', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, NULL, ? \
             WHERE NOT EXISTS (SELECT 1 FROM inbox_items WHERE dedupe_key = ?)",
        )
        .bind(&i.id)
        .bind(i.kind.as_str())
        .bind(i.group.as_str())
        .bind(&i.project_id)
        .bind(&i.ticket_id)
        .bind(&i.attempt_id)
        .bind(&i.session_id)
        .bind(&i.run_id)
        .bind(&i.step_id)
        .bind(&i.persona_id)
        .bind(&i.title)
        .bind(i.body.to_string())
        .bind(serde_json::to_string(&i.choices).unwrap_or_else(|_| "[]".into()))
        .bind(i.created_at)
        .bind(dedupe_key)
        .bind(dedupe_key)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_inbox_item(&i.id).await
    }

    /// The open items of one kind, oldest first.
    pub async fn open_inbox_items_of(&self, kind: InboxKind) -> Result<Vec<InboxItem>, StoreError> {
        let sql = format!(
            "SELECT {ITEM_COLUMNS} FROM inbox_items WHERE status = 'open' AND kind = ? ORDER BY created_at, id"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(kind.as_str()).fetch_all(&self.pool).await?;
        rows.iter().map(to_item).collect()
    }

    pub async fn get_inbox_item(&self, id: &str) -> Result<Option<InboxItem>, StoreError> {
        let sql = format!("SELECT {ITEM_COLUMNS} FROM inbox_items WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_item).transpose()
    }

    /// Open items, oldest first; with a project, its own and those with no project.
    pub async fn open_inbox_items(&self, project: Option<&str>) -> Result<Vec<InboxItem>, StoreError> {
        let rows = match project {
            Some(p) => {
                let sql = format!(
                    "SELECT {ITEM_COLUMNS} FROM inbox_items WHERE status = 'open' AND (project_id = ? OR project_id \
                     IS NULL) ORDER BY created_at, id"
                );
                sqlx::query(sqlx::AssertSqlSafe(sql)).bind(p).fetch_all(&self.pool).await?
            }
            None => {
                let sql =
                    format!("SELECT {ITEM_COLUMNS} FROM inbox_items WHERE status = 'open' ORDER BY created_at, id");
                sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?
            }
        };
        rows.iter().map(to_item).collect()
    }

    /// The open items of a step.
    pub async fn open_items_for_step(&self, step_id: &str) -> Result<Vec<InboxItem>, StoreError> {
        let sql = format!("SELECT {ITEM_COLUMNS} FROM inbox_items WHERE step_id = ? AND status = 'open'");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(step_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_item).collect()
    }

    /// Ends an open item (`resolved` with what was chosen, or `closed`). False when it was not open.
    pub async fn end_inbox_item(
        &self,
        id: &str,
        status: InboxStatus,
        resolution: &Value,
        now: i64,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE inbox_items SET status = ?, resolution = ?, resolved_at = ? WHERE id = ? AND status = 'open'",
        )
        .bind(status.as_str())
        .bind(resolution.to_string())
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Closes every open item of a step, or of a run's steps.
    pub async fn close_inbox_items(
        &self,
        step_id: Option<&str>,
        run_id: Option<&str>,
        now: i64,
    ) -> Result<u64, StoreError> {
        let r = sqlx::query(
            "UPDATE inbox_items SET status = 'closed', resolved_at = ? WHERE status = 'open' AND \
             ((? IS NOT NULL AND step_id = ?) OR (? IS NOT NULL AND run_id = ?))",
        )
        .bind(now)
        .bind(step_id)
        .bind(step_id)
        .bind(run_id)
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Deletes items resolved or closed before `before` (open ones stay however old). Returns how many went.
    pub async fn prune_inbox_items(&self, before: i64) -> Result<u64, StoreError> {
        let r = sqlx::query("DELETE FROM inbox_items WHERE status != 'open' AND resolved_at < ?")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Lets go of sessions that no longer exist (PDT-93): their open questions close, as nothing can answer them,
    /// and no item points at them any more (so "Open session" is never offered for one). Runs when a session is
    /// deleted and when the store opens (items left by sessions deleted before this).
    pub async fn release_gone_sessions(&self) -> Result<(), StoreError> {
        const GONE: &str = "session_id IS NOT NULL AND session_id NOT IN (SELECT id FROM sessions)";
        let mut tx = self.write_tx().await?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE inbox_items SET status = 'closed', resolution = '{{\"sessionGone\":true}}', \
             resolved_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
             WHERE status = 'open' AND kind = 'question' AND {GONE}"
        )))
        .execute(&mut *tx)
        .await?;
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE inbox_items SET session_id = NULL WHERE {GONE}")))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // ---- agent steps ----

    /// Records (or replaces) a step's binding.
    pub async fn put_agent_step(&self, a: &AgentStep) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO agent_steps (step_id, run_id, node_id, session_id, persona_id, persona_name, persona_version, \
             project_id, ticket_id, attempt_id, outcomes, state, turns, nudges, max_turns, auto_nudges, outcome, \
             summary, comments, by_user, created_at, updated_at, background_since, background, background_wait_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(step_id) DO UPDATE SET session_id = excluded.session_id, state = excluded.state, \
             turns = excluded.turns, nudges = excluded.nudges, outcome = excluded.outcome, \
             summary = excluded.summary, comments = excluded.comments, by_user = excluded.by_user, \
             updated_at = excluded.updated_at, background_since = excluded.background_since, \
             background = excluded.background",
        )
        .bind(&a.step_id)
        .bind(&a.run_id)
        .bind(&a.node_id)
        .bind(&a.session_id)
        .bind(&a.persona_id)
        .bind(&a.persona_name)
        .bind(i64::from(a.persona_version))
        .bind(&a.project_id)
        .bind(&a.ticket_id)
        .bind(&a.attempt_id)
        .bind(serde_json::to_string(&a.outcomes).unwrap_or_else(|_| "[]".into()))
        .bind(a.state.as_str())
        .bind(i64::from(a.turns))
        .bind(i64::from(a.nudges))
        .bind(i64::from(a.max_turns))
        .bind(i64::from(a.auto_nudges))
        .bind(&a.outcome)
        .bind(&a.summary)
        .bind(serde_json::to_string(&a.comments).unwrap_or_else(|_| "[]".into()))
        .bind(a.by_user)
        .bind(a.created_at)
        .bind(a.updated_at)
        .bind(a.background_since)
        .bind(serde_json::to_string(&a.background).unwrap_or_else(|_| "[]".into()))
        .bind(i64::try_from(a.background_wait_ms).unwrap_or(i64::MAX))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Forgets a step's binding (one whose session never started: the step starts over, PDT-129).
    pub async fn delete_agent_step(&self, step_id: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM agent_steps WHERE step_id = ?").bind(step_id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_agent_step(&self, step_id: &str) -> Result<Option<AgentStep>, StoreError> {
        let sql = format!("SELECT {STEP_COLUMNS} FROM agent_steps WHERE step_id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(step_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_step).transpose()
    }

    /// The session's step that is not done, newest first (a session runs one step at a time).
    pub async fn active_agent_step(&self, session_id: &str) -> Result<Option<AgentStep>, StoreError> {
        let sql = format!(
            "SELECT {STEP_COLUMNS} FROM agent_steps WHERE session_id = ? AND state != 'done' \
             ORDER BY created_at DESC LIMIT 1"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_step).transpose()
    }

    /// The session's newest step, done or not (a late `submit_outcome` after its step timed out, PDT-123).
    pub async fn latest_agent_step(&self, session_id: &str) -> Result<Option<AgentStep>, StoreError> {
        let sql =
            format!("SELECT {STEP_COLUMNS} FROM agent_steps WHERE session_id = ? ORDER BY created_at DESC LIMIT 1");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_step).transpose()
    }

    /// Whether a step that is not done works on the attempt (an agent is on it, PDT-104).
    pub async fn attempt_has_open_agent_step(&self, attempt_id: &str) -> Result<bool, StoreError> {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_steps WHERE attempt_id = ? AND state != 'done'")
            .bind(attempt_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(n > 0)
    }

    /// Whether an agent step works on the attempt now, outside run `except`: one not done, or a run's agent step not
    /// yet bound (waiting for a slot, or about to start) in an active run of the attempt (PDT-104).
    pub async fn attempt_agent_busy(&self, attempt_id: &str, except: &str) -> Result<bool, StoreError> {
        if self.attempt_has_open_agent_step(attempt_id).await? {
            return Ok(true);
        }
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_steps s JOIN workflow_runs r ON r.id = s.run_id \
             WHERE r.attempt_id = ? AND r.id != ? AND r.status IN ('running', 'waiting') AND s.kind = 'agent' \
             AND s.status IN ('pending', 'running', 'waiting')",
        )
        .bind(attempt_id)
        .bind(except)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Every step that is not done.
    pub async fn active_agent_steps(&self) -> Result<Vec<AgentStep>, StoreError> {
        let sql = format!("SELECT {STEP_COLUMNS} FROM agent_steps WHERE state != 'done' ORDER BY created_at");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_step).collect()
    }

    /// A run's steps of one node, oldest first (earlier passes).
    pub async fn agent_steps_of_node(&self, run_id: &str, node_id: &str) -> Result<Vec<AgentStep>, StoreError> {
        let sql =
            format!("SELECT {STEP_COLUMNS} FROM agent_steps WHERE run_id = ? AND node_id = ? ORDER BY created_at");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(run_id).bind(node_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_step).collect()
    }

    /// A ticket's steps that have an outcome, oldest first.
    pub async fn ticket_outcomes(&self, ticket_id: &str) -> Result<Vec<AgentStep>, StoreError> {
        let sql = format!(
            "SELECT {STEP_COLUMNS} FROM agent_steps WHERE ticket_id = ? AND outcome IS NOT NULL ORDER BY updated_at"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(ticket_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_step).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;
    use serde_json::json;

    fn item(id: &str, step: Option<&str>, at: i64) -> InboxItem {
        InboxItem {
            id: id.into(),
            kind: InboxKind::Gate,
            group: InboxGroup::Gate,
            status: InboxStatus::Open,
            project_id: Some("p".into()),
            ticket_id: None,
            ticket_key: None,
            ticket_title: None,
            attempt_id: None,
            session_id: None,
            run_id: Some("r1".into()),
            step_id: step.map(String::from),
            persona_id: None,
            persona_name: None,
            title: "Merge?".into(),
            body: json!({"question": "Merge?"}),
            choices: vec![InboxChoice::new("yes", "Yes", true)],
            resolution: Value::Null,
            created_at: at,
            resolved_at: None,
        }
    }

    #[tokio::test]
    async fn an_item_with_a_dedupe_key_is_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let notice = |id: &str| InboxItem { kind: InboxKind::Notice, group: InboxGroup::Notice, ..item(id, None, 1) };
        let made = store.add_inbox_item_once(&notice("n1"), "notice:a1:conflict:abc").await.unwrap();
        assert_eq!(made.unwrap().id, "n1");
        assert!(store.add_inbox_item_once(&notice("n2"), "notice:a1:conflict:abc").await.unwrap().is_none());
        assert!(store.add_inbox_item_once(&notice("n3"), "notice:a1:conflict:def").await.unwrap().is_some());
        let open: Vec<_> =
            store.open_inbox_items_of(InboxKind::Notice).await.unwrap().into_iter().map(|i| i.id).collect();
        assert_eq!(open, ["n1", "n3"]);
        // Closed, it is still the key's: the same notice doesn't come back.
        assert!(store.end_inbox_item("n1", InboxStatus::Closed, &json!({}), 2).await.unwrap());
        assert!(store.add_inbox_item_once(&notice("n4"), "notice:a1:conflict:abc").await.unwrap().is_none());
        assert_eq!(store.open_inbox_items_of(InboxKind::Notice).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn one_open_item_per_step_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let a = store.open_inbox_item(&item("i1", Some("s1"), 1)).await.unwrap();
        let again = store.open_inbox_item(&item("i2", Some("s1"), 2)).await.unwrap();
        assert_eq!(again.id, a.id, "the open one comes back");
        store.open_inbox_item(&item("q", None, 3)).await.unwrap();
        assert_eq!(store.open_inbox_items(Some("p")).await.unwrap().len(), 2);
        assert_eq!(store.open_inbox_items(Some("other")).await.unwrap().len(), 0);
        assert_eq!(a.choices[0].id, "yes");

        assert!(store.end_inbox_item("i1", InboxStatus::Resolved, &json!({"choice": "yes"}), 5).await.unwrap());
        assert!(!store.end_inbox_item("i1", InboxStatus::Resolved, &json!({}), 6).await.unwrap(), "only once");
        let got = store.get_inbox_item("i1").await.unwrap().unwrap();
        assert_eq!(
            (got.status, got.resolution["choice"].as_str(), got.resolved_at),
            (InboxStatus::Resolved, Some("yes"), Some(5))
        );
        store.open_inbox_item(&item("i3", Some("s1"), 7)).await.unwrap();
        assert_eq!(store.close_inbox_items(None, Some("r1"), 8).await.unwrap(), 2, "the run's items close");
        assert!(store.open_inbox_items(None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn old_ended_items_are_pruned_and_open_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        store.open_inbox_item(&item("done", Some("s1"), 1)).await.unwrap();
        store.open_inbox_item(&item("closed", Some("s2"), 1)).await.unwrap();
        store.open_inbox_item(&item("open", Some("s3"), 1)).await.unwrap();
        store.end_inbox_item("done", InboxStatus::Resolved, &json!({}), 10).await.unwrap();
        store.close_inbox_items(Some("s2"), None, 20).await.unwrap();
        assert_eq!(store.prune_inbox_items(10).await.unwrap(), 0, "resolved at `before`: kept");
        assert_eq!(store.prune_inbox_items(15).await.unwrap(), 1);
        assert!(store.get_inbox_item("done").await.unwrap().is_none());
        assert_eq!(store.prune_inbox_items(1000).await.unwrap(), 1);
        assert_eq!(store.open_inbox_items(None).await.unwrap().len(), 1, "the open one stays");
    }

    #[tokio::test]
    async fn a_deleted_sessions_questions_close_and_items_let_go_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        for id in ["s", "other"] {
            sqlx::query(
                "INSERT INTO sessions (id, project_id, kind, agent, title, cwd, status, created_at) \
                 VALUES (?, 'p', 'agent_tui', 'claude', 't', '/', 'running', 1)",
            )
            .bind(id)
            .execute(&store.pool)
            .await
            .unwrap();
        }
        let of = |id: &str, kind: InboxKind, session: &str| InboxItem {
            kind,
            session_id: Some(session.into()),
            run_id: None,
            ..item(id, None, 1)
        };
        store.open_inbox_item(&of("q", InboxKind::Question, "s")).await.unwrap();
        store.open_inbox_item(&of("stalled", InboxKind::Stalled, "s")).await.unwrap();
        store.open_inbox_item(&of("q2", InboxKind::Question, "other")).await.unwrap();
        assert!(store.delete_session("s").await.unwrap());

        let q = store.get_inbox_item("q").await.unwrap().unwrap();
        assert_eq!(
            (q.status, q.session_id, q.resolution["sessionGone"].as_bool()),
            (InboxStatus::Closed, None, Some(true))
        );
        assert!(q.resolved_at.is_some());
        let stalled = store.get_inbox_item("stalled").await.unwrap().unwrap();
        assert_eq!((stalled.status, stalled.session_id), (InboxStatus::Open, None), "a step's item stays open");
        let q2 = store.get_inbox_item("q2").await.unwrap().unwrap();
        assert_eq!(
            (q2.status, q2.session_id.as_deref()),
            (InboxStatus::Open, Some("other")),
            "other sessions' items stay"
        );
    }
}
