//! Remote attempts (PDT-632). On a remote instance: the sessions it runs for an owner (`exec_sessions`). On the owner:
//! each project's placement rules (`project_settings.placement`).

use crate::{Store, StoreError};

/// A session a remote instance runs for its owner: what the relay needs. The session's row is the owner's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecSession {
    pub session_id: String,
    pub link_id: String,
    pub agent: String,
    pub title: String,
    pub ticket_key: Option<String>,
    pub cwd: String,
    /// The session's pdt-mcp token, as the owner made it: the relay passes calls with it on to the owner.
    pub token: String,
    pub created_at: i64,
    pub ended_at: Option<i64>,
}

type Row = (String, String, String, String, Option<String>, String, String, i64, Option<i64>);

fn to_exec((session_id, link_id, agent, title, ticket_key, cwd, token, created_at, ended_at): Row) -> ExecSession {
    ExecSession { session_id, link_id, agent, title, ticket_key, cwd, token, created_at, ended_at }
}

const COLUMNS: &str = "session_id, link_id, agent, title, ticket_key, cwd, token, created_at, ended_at";

impl Store {
    /// Records (or, opened again, refreshes) a session this instance runs for an owner.
    pub async fn put_exec_session(&self, e: &ExecSession) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO exec_sessions (session_id, link_id, agent, title, ticket_key, cwd, token, created_at, ended_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL) ON CONFLICT(session_id) DO UPDATE SET \
             agent = excluded.agent, title = excluded.title, ticket_key = excluded.ticket_key, cwd = excluded.cwd, \
             token = excluded.token, ended_at = NULL",
        )
        .bind(&e.session_id)
        .bind(&e.link_id)
        .bind(&e.agent)
        .bind(&e.title)
        .bind(&e.ticket_key)
        .bind(&e.cwd)
        .bind(&e.token)
        .bind(e.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_exec_session(&self, session_id: &str) -> Result<Option<ExecSession>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM exec_sessions WHERE session_id = ?");
        let row: Option<Row> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(session_id).fetch_optional(&self.pool).await?;
        Ok(row.map(to_exec))
    }

    /// The sessions run for the owner on `link_id` (every link's when None), newest first.
    pub async fn list_exec_sessions(&self, link_id: Option<&str>) -> Result<Vec<ExecSession>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM exec_sessions WHERE (?1 IS NULL OR link_id = ?1) ORDER BY created_at DESC, session_id"
        );
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(link_id).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(to_exec).collect())
    }

    /// Its process ended.
    pub async fn end_exec_session(&self, session_id: &str, at: i64) -> Result<(), StoreError> {
        sqlx::query("UPDATE exec_sessions SET ended_at = COALESCE(ended_at, ?) WHERE session_id = ?")
            .bind(at)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_exec_session(&self, session_id: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM exec_sessions WHERE session_id = ?").bind(session_id).execute(&self.pool).await?;
        Ok(())
    }

    /// A project's placement rules on the owner; none until it is given some.
    pub async fn get_project_placement(
        &self,
        project_id: &str,
    ) -> Result<pdt_core::placement::PlacementSettings, StoreError> {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT placement FROM project_settings WHERE project_id = ?")
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?;
        match row.and_then(|(v,)| v) {
            None => Ok(Default::default()),
            Some(v) => serde_json::from_str(&v).map_err(|e| StoreError::Corrupt(format!("placement settings: {e}"))),
        }
    }

    pub async fn put_project_placement(
        &self,
        project_id: &str,
        s: &pdt_core::placement::PlacementSettings,
    ) -> Result<(), StoreError> {
        // The row gets the other settings as they read now, so a project with none keeps its defaults.
        let current = self.get_project_settings(project_id).await?;
        self.put_project_settings(project_id, &current).await?;
        sqlx::query("UPDATE project_settings SET placement = ? WHERE project_id = ?")
            .bind(serde_json::to_string(s).expect("placement settings serialize"))
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exec_sessions_are_kept_per_link_and_end() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("pdt.db")).await.unwrap();
        let (p, link) = crate::links::tests::remote("rp");
        store.insert_remote_project(&p, &link).await.unwrap();
        let e = ExecSession {
            session_id: "s".into(),
            link_id: link.link_id.clone(),
            agent: "claude".into(),
            title: "Coder · VAL-1".into(),
            ticket_key: Some("VAL-1".into()),
            cwd: "/c".into(),
            token: "s.t".into(),
            created_at: 1,
            ended_at: None,
        };
        store.put_exec_session(&e).await.unwrap();
        assert_eq!(store.get_exec_session("s").await.unwrap(), Some(e.clone()));
        store.end_exec_session("s", 5).await.unwrap();
        assert_eq!(store.list_exec_sessions(Some(&link.link_id)).await.unwrap()[0].ended_at, Some(5));
        store.put_exec_session(&e).await.unwrap();
        assert_eq!(store.get_exec_session("s").await.unwrap().unwrap().ended_at, None, "opened again");
        assert!(store.list_exec_sessions(Some("other")).await.unwrap().is_empty());
        store.delete_project("rp").await.unwrap();
        assert!(store.list_exec_sessions(None).await.unwrap().is_empty(), "gone with the remote project");
    }
}
