//! Conversation event log (structured agents spec §4.2).

use crate::{Store, StoreError};
use pdt_core::conversation::{AgentEvent, ConversationEvent, MAX_RAW, cap_text};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const COLUMNS: &str = "seq, pos, ts, item_id, turn_id, body";

fn to_event(r: &SqliteRow) -> Result<ConversationEvent, StoreError> {
    let body: String = r.try_get("body")?;
    let event: AgentEvent =
        serde_json::from_str(&body).map_err(|e| StoreError::Corrupt(format!("conversation event: {e}")))?;
    Ok(ConversationEvent {
        seq: r.try_get("seq")?,
        pos: r.try_get("pos")?,
        ts: r.try_get("ts")?,
        item_id: r.try_get("item_id")?,
        turn_id: r.try_get("turn_id")?,
        event,
    })
}

/// One event to add or update.
pub struct NewEvent<'a> {
    pub ts: i64,
    /// Identifies an item that changes; the row with this id is updated in place.
    pub item_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub event: &'a AgentEvent,
    /// The protocol frame(s) it came from (capped at 64 KB).
    pub raw: Option<&'a str>,
}

impl Store {
    /// Adds an event, or updates the row with the same `item_id` (it keeps its `pos` and gets a new `seq`).
    pub async fn upsert_session_event(
        &self,
        session_id: &str,
        e: NewEvent<'_>,
    ) -> Result<ConversationEvent, StoreError> {
        let body = serde_json::to_string(e.event).map_err(|err| StoreError::Corrupt(err.to_string()))?;
        let raw = e.raw.map(|r| cap_text(r, MAX_RAW));
        // IMMEDIATE: this reads then writes, and a deferred transaction that another writer gets ahead of cannot be
        // upgraded (SQLITE_BUSY_SNAPSHOT, which the busy timeout does not retry).
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let (seq, pos_next): (i64, i64) = sqlx::query_as(
            "SELECT COALESCE(MAX(seq), 0) + 1, COALESCE(MAX(pos), 0) + 1 FROM session_events WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?;
        let existing: Option<(i64,)> = match e.item_id {
            Some(item) => {
                sqlx::query_as("SELECT pos FROM session_events WHERE session_id = ? AND item_id = ?")
                    .bind(session_id)
                    .bind(item)
                    .fetch_optional(&mut *tx)
                    .await?
            }
            None => None,
        };
        let pos = match existing {
            Some((pos,)) => {
                sqlx::query(
                    "UPDATE session_events SET seq = ?, ts = ?, kind = ?, turn_id = COALESCE(?, turn_id), body = ?, \
                     raw = COALESCE(?, raw) WHERE session_id = ? AND item_id = ?",
                )
                .bind(seq)
                .bind(e.ts)
                .bind(e.event.kind())
                .bind(e.turn_id)
                .bind(&body)
                .bind(&raw)
                .bind(session_id)
                .bind(e.item_id)
                .execute(&mut *tx)
                .await?;
                pos
            }
            None => {
                sqlx::query(
                    "INSERT INTO session_events (session_id, seq, pos, ts, kind, item_id, turn_id, body, raw) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(session_id)
                .bind(seq)
                .bind(pos_next)
                .bind(e.ts)
                .bind(e.event.kind())
                .bind(e.item_id)
                .bind(e.turn_id)
                .bind(&body)
                .bind(&raw)
                .execute(&mut *tx)
                .await?;
                pos_next
            }
        };
        let turn_id: Option<String> =
            sqlx::query_scalar("SELECT turn_id FROM session_events WHERE session_id = ? AND seq = ?")
                .bind(session_id)
                .bind(seq)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(ConversationEvent {
            seq,
            pos,
            ts: e.ts,
            item_id: e.item_id.map(String::from),
            turn_id,
            event: e.event.clone(),
        })
    }

    /// Events with `seq > after`, oldest first, at most `limit`.
    pub async fn list_session_events(
        &self,
        session_id: &str,
        after: i64,
        limit: i64,
    ) -> Result<Vec<ConversationEvent>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM session_events WHERE session_id = ? AND seq > ? ORDER BY seq LIMIT ?");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(session_id)
            .bind(after)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(to_event).collect()
    }

    /// A session's last `limit` events, oldest first.
    pub async fn recent_session_events(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationEvent>, StoreError> {
        let sql = format!(
            "SELECT * FROM (SELECT {COLUMNS} FROM session_events WHERE session_id = ? ORDER BY pos DESC LIMIT ?) \
             ORDER BY pos"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id).bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(to_event).collect()
    }

    /// Every event of one kind, in conversation order.
    pub async fn session_events_of_kind(
        &self,
        session_id: &str,
        kind: &str,
    ) -> Result<Vec<ConversationEvent>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM session_events WHERE session_id = ? AND kind = ? ORDER BY pos");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id).bind(kind).fetch_all(&self.pool).await?;
        rows.iter().map(to_event).collect()
    }

    /// The stored protocol frames of an event (debugging).
    pub async fn session_event_raw(&self, session_id: &str, seq: i64) -> Result<Option<String>, StoreError> {
        let raw: Option<Option<String>> =
            sqlx::query_scalar("SELECT raw FROM session_events WHERE session_id = ? AND seq = ?")
                .bind(session_id)
                .bind(seq)
                .fetch_optional(&self.pool)
                .await?;
        Ok(raw.flatten())
    }

    /// The agent turn ids the log has (for importing only the turns it lacks).
    pub async fn session_turn_ids(&self, session_id: &str) -> Result<std::collections::HashSet<String>, StoreError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT turn_id FROM session_events WHERE session_id = ? AND turn_id IS NOT NULL")
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(t,)| t).collect())
    }

    /// The newest `seq` (0 when the conversation is empty).
    pub async fn last_session_event_seq(&self, session_id: &str) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM session_events WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::NewEvent;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::conversation::{AgentEvent, ToolStatus};
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};

    fn tool(status: ToolStatus) -> AgentEvent {
        AgentEvent::ToolCall {
            id: "t1".into(),
            tool: "Bash".into(),
            title: "ls".into(),
            input: serde_json::json!({"command": "ls"}),
            status,
            output: None,
            exit_code: None,
            diff: None,
            parent_id: None,
        }
    }

    #[tokio::test]
    async fn items_update_in_place_with_a_new_seq() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let s = Session {
            id: "s1".into(),
            project_id: "p".into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            bypass_permissions: false,
            title: "t".into(),
            cwd: "/".into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at: 1,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: None,
            activity: None,
            attention_at: None,
            last_viewed_at: None,
            unseen: false,
            attempt_id: None,
            ticket: None,
            mode: pdt_core::conversation::SessionMode::Conversation,
            model: Some("opus".into()),
            permission_mode: Some(pdt_core::conversation::PermissionMode::Plan),
            persona: None,
        };
        store.insert_session(&s).await.unwrap();
        let back = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!((back.mode, back.model.as_deref()), (s.mode, Some("opus")));
        assert_eq!(back.permission_mode, s.permission_mode);
        let running = tool(ToolStatus::Running);
        let note = AgentEvent::Reasoning { summary: "hm".into() };
        let ev = |item: Option<&'static str>, turn: Option<&'static str>, e: &'static AgentEvent| NewEvent {
            ts: 5,
            item_id: item,
            turn_id: turn,
            event: e,
            raw: Some("{}"),
        };
        let running: &'static AgentEvent = Box::leak(Box::new(running));
        let note: &'static AgentEvent = Box::leak(Box::new(note));
        let done: &'static AgentEvent = Box::leak(Box::new(tool(ToolStatus::Done)));
        let a = store.upsert_session_event("s1", ev(Some("t:t1"), Some("turn1"), running)).await.unwrap();
        let b = store.upsert_session_event("s1", ev(None, None, note)).await.unwrap();
        let c = store.upsert_session_event("s1", ev(Some("t:t1"), None, done)).await.unwrap();
        assert_eq!((a.seq, a.pos), (1, 1));
        assert_eq!((b.seq, b.pos), (2, 2));
        assert_eq!((c.seq, c.pos), (3, 1), "same row and place, new seq");
        assert_eq!(c.turn_id.as_deref(), Some("turn1"), "keeps its turn");
        let all = store.list_session_events("s1", 0, 100).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].event, *done);
        assert_eq!(store.list_session_events("s1", 2, 100).await.unwrap().len(), 1, "after seq 2 includes the change");
        assert_eq!(store.session_events_of_kind("s1", "tool_call").await.unwrap().len(), 1);
        assert_eq!(store.last_session_event_seq("s1").await.unwrap(), 3);
        assert_eq!(store.last_session_event_seq("nope").await.unwrap(), 0);
        assert_eq!(store.session_event_raw("s1", 3).await.unwrap().as_deref(), Some("{}"));
        store.set_session_model("s1", None).await.unwrap();
        store.set_session_permission_mode("s1", pdt_core::conversation::PermissionMode::Bypass).await.unwrap();
        let back = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!(back.model, None);
        assert!(back.bypass_permissions);
        store.delete_session("s1").await.unwrap();
        assert!(store.list_session_events("s1", 0, 100).await.unwrap().is_empty(), "events go with the session");
    }
}
