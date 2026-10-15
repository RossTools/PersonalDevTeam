use crate::{Store, StoreError};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const COLUMNS: &str = "id, project_id, worktree_id, kind, agent, title, cwd, status, exit_code, created_at, ended_at, \
                       external_session_id, started_at";

fn to_session(r: &SqliteRow) -> Result<Session, StoreError> {
    let kind: String = r.try_get("kind")?;
    let agent: Option<String> = r.try_get("agent")?;
    let status: String = r.try_get("status")?;
    let corrupt = |what: &str, v: &str| StoreError::Corrupt(format!("unknown session {what} {v:?}"));
    Ok(Session {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        worktree_id: r.try_get("worktree_id")?,
        kind: SessionKind::parse(&kind).ok_or_else(|| corrupt("kind", &kind))?,
        agent: match agent {
            Some(a) => Some(AgentKind::parse(&a).ok_or_else(|| corrupt("agent", &a))?),
            None => None,
        },
        title: r.try_get("title")?,
        cwd: r.try_get("cwd")?,
        status: SessionStatus::parse(&status).ok_or_else(|| corrupt("status", &status))?,
        exit_code: r.try_get("exit_code")?,
        created_at: r.try_get("created_at")?,
        ended_at: r.try_get("ended_at")?,
        external_session_id: r.try_get("external_session_id")?,
        started_at: r.try_get("started_at")?,
    })
}

impl Store {
    pub async fn insert_session(&self, s: &Session) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sessions (id, project_id, worktree_id, kind, agent, title, cwd, status, exit_code, created_at, ended_at, \
             external_session_id, started_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&s.id)
        .bind(&s.project_id)
        .bind(&s.worktree_id)
        .bind(s.kind.as_str())
        .bind(s.agent.map(AgentKind::as_str))
        .bind(&s.title)
        .bind(&s.cwd)
        .bind(s.status.as_str())
        .bind(s.exit_code)
        .bind(s.created_at)
        .bind(s.ended_at)
        .bind(&s.external_session_id)
        .bind(s.started_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_sessions(&self, project_id: &str) -> Result<Vec<Session>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM sessions WHERE project_id = ? ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_session).collect()
    }

    /// Sessions with the given status, oldest first.
    pub async fn list_sessions_by_status(&self, status: SessionStatus) -> Result<Vec<Session>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM sessions WHERE status = ? ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(status.as_str()).fetch_all(&self.pool).await?;
        rows.iter().map(to_session).collect()
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM sessions WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_session).transpose()
    }

    /// Ends a session that is still `running`. Returns false when it was not running (already ended).
    pub async fn end_session(
        &self,
        id: &str,
        status: SessionStatus,
        exit_code: Option<i32>,
        ended_at: i64,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE sessions SET status = ?, exit_code = ?, ended_at = ? WHERE id = ? AND status = 'running'",
        )
        .bind(status.as_str())
        .bind(exit_code)
        .bind(ended_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Marks a session running again for a new spawn at `started_at`.
    pub async fn set_session_running(&self, id: &str, started_at: i64) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE sessions SET status = 'running', exit_code = NULL, ended_at = NULL, started_at = ? WHERE id = ?",
        )
        .bind(started_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// The agent conversation ids already bound to sessions other than `except`.
    pub async fn claimed_external_session_ids(&self, except: &str) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT external_session_id FROM sessions WHERE external_session_id IS NOT NULL AND id != ?",
        )
        .bind(except)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(v,)| v).collect())
    }

    /// Binds an agent conversation id to a session that has none. Returns false when the session already has one,
    /// does not exist, or another session owns that conversation.
    pub async fn bind_external_session_id(&self, id: &str, external_id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE sessions SET external_session_id = ? WHERE id = ? AND external_session_id IS NULL \
             AND NOT EXISTS (SELECT 1 FROM sessions WHERE external_session_id = ?)",
        )
        .bind(external_id)
        .bind(id)
        .bind(external_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn rename_session(&self, id: &str, title: &str) -> Result<bool, StoreError> {
        let r =
            sqlx::query("UPDATE sessions SET title = ? WHERE id = ?").bind(title).bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_session(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM sessions WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Marks every `running` session as `interrupted` (pdtd stopped or restarted). Returns how many changed.
    pub async fn mark_running_interrupted(&self, ended_at: i64) -> Result<u64, StoreError> {
        let r = sqlx::query("UPDATE sessions SET status = 'interrupted', ended_at = ? WHERE status = 'running'")
            .bind(ended_at)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use crate::StoreError;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};

    fn session(id: &str, created_at: i64) -> Session {
        Session {
            id: id.into(),
            project_id: "p".into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Codex),
            title: "Codex".into(),
            cwd: "C:\\r Ã©".into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at,
            ended_at: None,
            external_session_id: None,
            started_at: Some(created_at),
        }
    }

    #[tokio::test]
    async fn session_roundtrip_in_creation_order() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("b", 20)).await.unwrap();
        store.insert_session(&session("a", 10)).await.unwrap();
        let ids: Vec<_> = store.list_sessions("p").await.unwrap().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(store.get_session("a").await.unwrap(), Some(session("a", 10)));
        assert_eq!(store.get_session("zz").await.unwrap(), None);
    }

    #[tokio::test]
    async fn end_session_only_touches_running_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        assert!(store.end_session("a", SessionStatus::Finished, Some(3), 99).await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.status, s.exit_code, s.ended_at), (SessionStatus::Finished, Some(3), Some(99)));
        assert!(!store.end_session("a", SessionStatus::Interrupted, None, 100).await.unwrap(), "already ended");
        assert!(store.set_session_running("a", 7).await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.status, s.exit_code, s.ended_at), (SessionStatus::Running, None, None));
    }

    #[tokio::test]
    async fn rename_session_updates_title() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        assert!(store.rename_session("a", "New name").await.unwrap());
        assert_eq!(store.get_session("a").await.unwrap().unwrap().title, "New name");
        assert!(!store.rename_session("zz", "x").await.unwrap());
    }

    #[tokio::test]
    async fn mark_running_interrupted_counts_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        store.insert_session(&session("b", 2)).await.unwrap();
        store.end_session("b", SessionStatus::Finished, Some(0), 5).await.unwrap();
        assert_eq!(store.mark_running_interrupted(7).await.unwrap(), 1);
        let a = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((a.status, a.ended_at), (SessionStatus::Interrupted, Some(7)));
        assert_eq!(store.get_session("b").await.unwrap().unwrap().status, SessionStatus::Finished);
    }

    #[tokio::test]
    async fn external_ids_bind_once_and_are_unique() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        for (id, at) in [("a", 1), ("b", 2)] {
            store.insert_session(&session(id, at)).await.unwrap();
        }
        assert!(store.bind_external_session_id("a", "conv-1").await.unwrap());
        assert!(!store.bind_external_session_id("a", "conv-2").await.unwrap(), "already bound");
        assert!(!store.bind_external_session_id("b", "conv-1").await.unwrap(), "owned by another session");
        assert!(store.bind_external_session_id("b", "conv-3").await.unwrap());
        assert_eq!(store.claimed_external_session_ids("a").await.unwrap(), ["conv-3"]);
        assert_eq!(store.get_session("a").await.unwrap().unwrap().external_session_id.as_deref(), Some("conv-1"));
    }

    #[tokio::test]
    async fn set_session_running_records_the_spawn_time() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        store.end_session("a", SessionStatus::Finished, Some(0), 5).await.unwrap();
        assert!(store.set_session_running("a", 99).await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.status, s.started_at, s.ended_at), (SessionStatus::Running, Some(99), None));
    }

    #[tokio::test]
    async fn list_sessions_by_status_is_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        for (id, at) in [("c", 3), ("a", 1), ("b", 2)] {
            store.insert_session(&session(id, at)).await.unwrap();
        }
        store.end_session("b", SessionStatus::Finished, Some(0), 5).await.unwrap();
        store.mark_running_interrupted(7).await.unwrap();
        let ids = |v: Vec<Session>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(ids(store.list_sessions_by_status(SessionStatus::Interrupted).await.unwrap()), ["a", "c"]);
        assert_eq!(ids(store.list_sessions_by_status(SessionStatus::Finished).await.unwrap()), ["b"]);
        assert!(store.list_sessions_by_status(SessionStatus::Running).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_session_and_corrupt_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        assert!(store.delete_session("a").await.unwrap());
        assert!(!store.delete_session("a").await.unwrap());
        sqlx::query(
            "INSERT INTO sessions (id, project_id, kind, title, cwd, status, created_at) \
             VALUES ('x', 'p', 'teleporter', 't', '/', 'running', 1)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        assert!(matches!(store.get_session("x").await, Err(StoreError::Corrupt(_))));
    }

    #[tokio::test]
    async fn session_in_a_worktree_roundtrips_and_needs_a_real_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let w = pdt_core::worktree::Worktree {
            id: "w".into(),
            project_id: "p".into(),
            branch: "b".into(),
            path: "/w".into(),
            base_commit: "c".into(),
            created_at: 1,
        };
        store.insert_worktree(&w).await.unwrap();
        let mut s = session("a", 1);
        s.worktree_id = Some("w".into());
        store.insert_session(&s).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap(), Some(s));
        let mut orphan = session("b", 2);
        orphan.worktree_id = Some("missing".into());
        assert!(store.insert_session(&orphan).await.is_err(), "foreign key enforced");
    }
}
