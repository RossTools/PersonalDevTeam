use crate::{Store, StoreError};
use pdt_core::activity::Activity;
use pdt_core::conversation::{BackgroundTask, PermissionMode, SessionMode};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use pdt_core::ticket::{SessionTicket, format_key};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// Sessions with their ticket (id and key: the attempt's, else the one they were started on), for `WHERE s.…` clauses.
const SELECT: &str = "SELECT s.id, s.project_id, s.worktree_id, s.kind, s.agent, s.bypass_permissions, s.title, s.cwd, \
                      s.status, s.exit_code, s.created_at, s.ended_at, s.external_session_id, s.fresh_started_at, \
                      s.activity, s.attention_at, s.last_viewed_at, s.attempt_id, s.mode, s.model, s.permission_mode, \
                      s.persona_id, s.persona_version, s.step_id, s.background_tasks, pe.name AS persona_name, \
                      t.id AS ticket_id, \
                      COALESCE(p.key, '') AS ticket_pkey, t.number AS ticket_number \
                      FROM sessions s LEFT JOIN attempts a ON a.id = s.attempt_id \
                      LEFT JOIN tickets t ON t.id = COALESCE(a.ticket_id, s.ticket_id) LEFT JOIN projects p ON p.id = t.project_id \
                      LEFT JOIN personas pe ON pe.id = s.persona_id";

fn to_session(r: &SqliteRow) -> Result<Session, StoreError> {
    let kind: String = r.try_get("kind")?;
    let agent: Option<String> = r.try_get("agent")?;
    let status: String = r.try_get("status")?;
    let activity: Option<String> = r.try_get("activity")?;
    let attention_at: Option<i64> = r.try_get("attention_at")?;
    let last_viewed_at: Option<i64> = r.try_get("last_viewed_at")?;
    let corrupt = |what: &str, v: &str| StoreError::Corrupt(format!("unknown session {what} {v:?}"));
    let mode: String = r.try_get("mode")?;
    let permission_mode: Option<String> = r.try_get("permission_mode")?;
    let ticket_id: Option<String> = r.try_get("ticket_id")?;
    let ticket = match ticket_id {
        Some(id) => {
            let pkey: String = r.try_get("ticket_pkey")?;
            let number = u32::try_from(r.try_get::<i64, _>("ticket_number")?).unwrap_or(0);
            Some(SessionTicket { id, key: format_key(&pkey, number) })
        }
        None => None,
    };
    let persona_id: Option<String> = r.try_get("persona_id")?;
    let persona = match persona_id {
        Some(id) => Some(pdt_core::session::SessionPersona {
            name: r.try_get::<Option<String>, _>("persona_name")?.unwrap_or_default(),
            version: u32::try_from(r.try_get::<Option<i64>, _>("persona_version")?.unwrap_or(0)).unwrap_or(0),
            step_id: r.try_get("step_id")?,
            id,
        }),
        None => None,
    };
    Ok(Session {
        persona,
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        worktree_id: r.try_get("worktree_id")?,
        kind: SessionKind::parse(&kind).ok_or_else(|| corrupt("kind", &kind))?,
        agent: match agent {
            Some(a) => Some(AgentKind::parse_stored(&a).ok_or_else(|| corrupt("agent", &a))?),
            None => None,
        },
        bypass_permissions: r.try_get("bypass_permissions")?,
        title: r.try_get("title")?,
        cwd: r.try_get("cwd")?,
        status: SessionStatus::parse(&status).ok_or_else(|| corrupt("status", &status))?,
        exit_code: r.try_get("exit_code")?,
        created_at: r.try_get("created_at")?,
        ended_at: r.try_get("ended_at")?,
        external_session_id: r.try_get("external_session_id")?,
        fresh_started_at: r.try_get("fresh_started_at")?,
        activity: match activity {
            Some(a) => Some(Activity::parse(&a).ok_or_else(|| corrupt("activity", &a))?),
            None => None,
        },
        attention_at,
        last_viewed_at,
        unseen: pdt_core::activity::is_unseen(attention_at, last_viewed_at),
        attempt_id: r.try_get("attempt_id")?,
        ticket,
        mode: SessionMode::parse(&mode).ok_or_else(|| corrupt("mode", &mode))?,
        model: r.try_get("model")?,
        permission_mode: match permission_mode {
            Some(m) => Some(PermissionMode::parse(&m).ok_or_else(|| corrupt("permission mode", &m))?),
            None => None,
        },
        // Only a running session's: an interrupted one keeps them for the notice its agent gets (PDT-311).
        background: if status == "running" {
            parse_tasks(r.try_get::<Option<String>, _>("background_tasks")?.as_deref())
        } else {
            vec![]
        },
    })
}

impl Store {
    pub async fn insert_session(&self, s: &Session) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sessions (id, project_id, worktree_id, kind, agent, bypass_permissions, title, cwd, status, exit_code, created_at, ended_at, \
             external_session_id, fresh_started_at, activity, attention_at, last_viewed_at, attempt_id, mode, model, \
             permission_mode, persona_id, persona_version, step_id, ticket_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&s.id)
        .bind(&s.project_id)
        .bind(&s.worktree_id)
        .bind(s.kind.as_str())
        .bind(s.agent.map(AgentKind::as_str))
        .bind(s.bypass_permissions)
        .bind(&s.title)
        .bind(&s.cwd)
        .bind(s.status.as_str())
        .bind(s.exit_code)
        .bind(s.created_at)
        .bind(s.ended_at)
        .bind(&s.external_session_id)
        .bind(s.fresh_started_at)
        .bind(s.activity.map(Activity::as_str))
        .bind(s.attention_at)
        .bind(s.last_viewed_at)
        .bind(&s.attempt_id)
        .bind(s.mode.as_str())
        .bind(&s.model)
        .bind(s.permission_mode.map(PermissionMode::as_str))
        .bind(s.persona.as_ref().map(|p| p.id.clone()))
        .bind(s.persona.as_ref().map(|p| i64::from(p.version)))
        .bind(s.persona.as_ref().and_then(|p| p.step_id.clone()))
        .bind(s.ticket.as_ref().map(|t| t.id.clone()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_sessions(&self, project_id: &str) -> Result<Vec<Session>, StoreError> {
        let sql = format!("{SELECT} WHERE s.project_id = ? ORDER BY s.created_at, s.id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_session).collect()
    }

    /// An attempt's sessions, oldest first.
    pub async fn list_attempt_sessions(&self, attempt_id: &str) -> Result<Vec<Session>, StoreError> {
        let sql = format!("{SELECT} WHERE s.attempt_id = ? ORDER BY s.created_at, s.id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(attempt_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_session).collect()
    }

    /// Sessions with the given status, oldest first.
    pub async fn list_sessions_by_status(&self, status: SessionStatus) -> Result<Vec<Session>, StoreError> {
        let sql = format!("{SELECT} WHERE s.status = ? ORDER BY s.created_at, s.id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(status.as_str()).fetch_all(&self.pool).await?;
        rows.iter().map(to_session).collect()
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<Session>, StoreError> {
        let sql = format!("{SELECT} WHERE s.id = ?");
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
            "UPDATE sessions SET status = ?, exit_code = ?, ended_at = ?, activity = NULL \
             WHERE id = ? AND status = 'running'",
        )
        .bind(status.as_str())
        .bind(exit_code)
        .bind(ended_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn set_session_running(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET status = 'running', exit_code = NULL, ended_at = NULL WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Makes a session with no ticket work on an attempt (Make ticket's "Start from this chat"). False if the session
    /// is gone or already has an attempt.
    pub async fn set_session_attempt(&self, id: &str, attempt_id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET attempt_id = ? WHERE id = ? AND attempt_id IS NULL")
            .bind(attempt_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Records that a Codex session was launched fresh at `at` (ms).
    pub async fn set_fresh_started_at(&self, id: &str, at: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET fresh_started_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Every Codex session, whatever its status.
    pub async fn list_codex_sessions(&self) -> Result<Vec<Session>, StoreError> {
        let sql = format!("{SELECT} WHERE s.agent = 'codex' ORDER BY s.created_at, s.id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_session).collect()
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

    /// Sets a conversation session's agent conversation id (Codex's thread), replacing one that could not be resumed.
    pub async fn set_external_session_id(&self, id: &str, external_id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET external_session_id = ? WHERE id = ?")
            .bind(external_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
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
        .await;
        match r {
            Ok(r) => Ok(r.rows_affected() > 0),
            // Lost a race with a concurrent bind of the same conversation (the unique index backs the check above).
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Sets the activity; `attention_at` (when Some) records that it now needs the user's attention.
    pub async fn update_activity(
        &self,
        id: &str,
        activity: Option<Activity>,
        attention_at: Option<i64>,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET activity = ?, attention_at = COALESCE(?, attention_at) WHERE id = ?")
            .bind(activity.map(Activity::as_str))
            .bind(attention_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Records that the user viewed the session at `at` (ms), on any device.
    pub async fn mark_viewed(&self, id: &str, at: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET last_viewed_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// A conversation's model (None: the agent's default).
    pub async fn set_session_model(&self, id: &str, model: Option<&str>) -> Result<bool, StoreError> {
        let r =
            sqlx::query("UPDATE sessions SET model = ? WHERE id = ?").bind(model).bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// A conversation's permission mode (bypass also sets `bypass_permissions`, which a terminal launch reads).
    pub async fn set_session_permission_mode(&self, id: &str, mode: PermissionMode) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET permission_mode = ?, bypass_permissions = ? WHERE id = ?")
            .bind(mode.as_str())
            .bind(mode == PermissionMode::Bypass)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Switches how an agent session is driven, with the transcript entry PDT's log ends at (spec §8.3; None once
    /// imported).
    pub async fn switch_session_mode(
        &self,
        id: &str,
        mode: SessionMode,
        history_from: Option<&str>,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE sessions SET mode = ?, history_from = ? WHERE id = ?")
            .bind(mode.as_str())
            .bind(history_from)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// The transcript entry PDT's log ends at, while the agent's TUI has the session.
    pub async fn history_from(&self, id: &str) -> Result<Option<String>, StoreError> {
        let v: Option<Option<String>> = sqlx::query_scalar("SELECT history_from FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(v.flatten())
    }

    /// Sessions made before permission modes existed get theirs from `bypass_permissions`.
    pub(crate) async fn backfill_permission_modes(&self) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE sessions SET permission_mode = CASE WHEN bypass_permissions THEN 'bypass' ELSE 'ask' END \
             WHERE permission_mode IS NULL AND kind = 'agent_tui'",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rename_session(&self, id: &str, title: &str) -> Result<bool, StoreError> {
        let r =
            sqlx::query("UPDATE sessions SET title = ? WHERE id = ?").bind(title).bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Deletes a session; the Inbox lets go of it (`release_gone_sessions`).
    pub async fn delete_session(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM sessions WHERE id = ?").bind(id).execute(&self.pool).await?;
        self.release_gone_sessions().await?;
        Ok(r.rows_affected() > 0)
    }

    /// Marks every `running` session as `interrupted` (pdtd stopped or restarted). Returns how many changed.
    pub async fn mark_running_interrupted(&self, ended_at: i64) -> Result<u64, StoreError> {
        let r = sqlx::query(
            "UPDATE sessions SET status = 'interrupted', ended_at = ?, activity = NULL WHERE status = 'running'",
        )
        .bind(ended_at)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// A conversation's background tasks as the agent last reported them (PDT-311): kept so a restart knows what it
    /// stopped, and shown on the session (PDT-332). None run: cleared. When the last ones end while the agent is idle,
    /// its work has finished now: `at` becomes its attention time, so it reads as unseen (Done), as a turn ending does.
    pub async fn set_session_background(
        &self,
        id: &str,
        tasks: &[BackgroundTask],
        at: i64,
    ) -> Result<bool, StoreError> {
        let json = (!tasks.is_empty()).then(|| serde_json::to_string(tasks).expect("tasks serialize"));
        let r = sqlx::query(
            "UPDATE sessions SET attention_at = CASE WHEN ?1 IS NULL AND background_tasks IS NOT NULL \
             AND activity = 'idle' THEN ?2 ELSE attention_at END, background_tasks = ?1 WHERE id = ?3",
        )
        .bind(json)
        .bind(at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// The previous pdtd stopped every session's background tasks: they move to `stopped_background` (added to any
    /// not told yet), until the agent is told. Returns the sessions that had some.
    pub async fn retire_session_background(&self) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, background_tasks, stopped_background FROM sessions WHERE background_tasks IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut ids = Vec::new();
        for (id, running, stopped) in rows {
            let mut all: Vec<BackgroundTask> = parse_tasks(stopped.as_deref());
            for t in parse_tasks(Some(&running)) {
                if !all.iter().any(|k| k.id == t.id) {
                    all.push(t);
                }
            }
            let json = (!all.is_empty()).then(|| serde_json::to_string(&all).expect("tasks serialize"));
            sqlx::query("UPDATE sessions SET background_tasks = NULL, stopped_background = ? WHERE id = ?")
                .bind(json)
                .bind(&id)
                .execute(&self.pool)
                .await?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// The background tasks a restart stopped that the session's agent has not been told about.
    pub async fn stopped_background(&self, id: &str) -> Result<Vec<BackgroundTask>, StoreError> {
        let v: Option<Option<String>> = sqlx::query_scalar("SELECT stopped_background FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(parse_tasks(v.flatten().as_deref()))
    }

    /// Sets (or, empty, clears) what `stopped_background` returns.
    pub async fn set_stopped_background(&self, id: &str, tasks: &[BackgroundTask]) -> Result<bool, StoreError> {
        let json = (!tasks.is_empty()).then(|| serde_json::to_string(tasks).expect("tasks serialize"));
        let r = sqlx::query("UPDATE sessions SET stopped_background = ? WHERE id = ?")
            .bind(json)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}

/// A stored task list; an unreadable one is none (it only ever informs a notice).
fn parse_tasks(json: Option<&str>) -> Vec<BackgroundTask> {
    json.and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::StoreError;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};

    pub(crate) fn session(id: &str, created_at: i64) -> Session {
        Session {
            id: id.into(),
            project_id: "p".into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Codex),
            bypass_permissions: false,
            title: "Codex".into(),
            cwd: "C:\\r é".into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: Some(created_at),
            activity: None,
            attention_at: None,
            last_viewed_at: None,
            unseen: false,
            attempt_id: None,
            ticket: None,
            mode: pdt_core::conversation::SessionMode::Terminal,
            model: None,
            permission_mode: None,
            persona: None,
            background: vec![],
        }
    }

    #[tokio::test]
    async fn activity_attention_and_viewing() {
        use pdt_core::activity::Activity;
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        store.update_activity("a", Some(Activity::Working), None).await.unwrap();
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.activity, s.unseen), (Some(Activity::Working), false));
        store.update_activity("a", Some(Activity::Idle), Some(50)).await.unwrap();
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.activity, s.attention_at, s.unseen), (Some(Activity::Idle), Some(50), true));
        store.update_activity("a", Some(Activity::Working), None).await.unwrap();
        assert_eq!(
            store.get_session("a").await.unwrap().unwrap().attention_at,
            Some(50),
            "kept until the next attention"
        );
        assert!(store.mark_viewed("a", 60).await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.last_viewed_at, s.unseen), (Some(60), false));
        store.update_activity("a", None, None).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap().unwrap().activity, None);
        assert!(!store.mark_viewed("zz", 1).await.unwrap());
        store.update_activity("a", Some(Activity::Working), None).await.unwrap();
        store.end_session("a", SessionStatus::Finished, Some(0), 70).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap().unwrap().activity, None, "ended sessions have none");
        store.set_session_running("a").await.unwrap();
        store.update_activity("a", Some(Activity::Working), None).await.unwrap();
        store.mark_running_interrupted(80).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap().unwrap().activity, None, "interrupted ones neither");
        sqlx::query("UPDATE sessions SET activity = 'dancing' WHERE id = 'a'").execute(&store.pool).await.unwrap();
        assert!(matches!(store.get_session("a").await, Err(StoreError::Corrupt(_))));
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
        assert!(store.set_session_running("a").await.unwrap());
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
    async fn mode_and_history_anchor_switch() {
        use pdt_core::conversation::SessionMode;
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        assert_eq!(store.history_from("a").await.unwrap(), None);
        assert!(store.switch_session_mode("a", SessionMode::Conversation, Some("uuid-9")).await.unwrap());
        assert_eq!(store.get_session("a").await.unwrap().unwrap().mode, SessionMode::Conversation);
        assert_eq!(store.history_from("a").await.unwrap().as_deref(), Some("uuid-9"));
        assert!(store.switch_session_mode("a", SessionMode::Terminal, None).await.unwrap());
        assert_eq!(store.history_from("a").await.unwrap(), None);
        assert!(!store.switch_session_mode("zz", SessionMode::Terminal, None).await.unwrap());
    }

    #[tokio::test]
    async fn background_tasks_are_kept_and_retired_by_a_restart() {
        use pdt_core::conversation::BackgroundTask;
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        store.insert_session(&session("b", 2)).await.unwrap();
        let task =
            |id: &str| BackgroundTask { id: id.into(), description: format!("task {id}"), kind: "local_bash".into() };
        store.set_session_background("a", &[task("t1")], 1).await.unwrap();
        store.set_session_background("b", &[task("t2")], 1).await.unwrap();
        store.set_session_background("b", &[], 1).await.unwrap();
        assert_eq!(store.retire_session_background().await.unwrap(), ["a"], "b's ended before the restart");
        assert_eq!(store.stopped_background("a").await.unwrap(), [task("t1")]);
        assert!(store.stopped_background("b").await.unwrap().is_empty());
        // Another restart before the agent was told adds to what it is told.
        store.set_session_background("a", &[task("t1"), task("t3")], 1).await.unwrap();
        store.retire_session_background().await.unwrap();
        assert_eq!(store.stopped_background("a").await.unwrap(), [task("t1"), task("t3")]);
        assert!(store.retire_session_background().await.unwrap().is_empty(), "nothing runs any more");
        store.set_stopped_background("a", &[]).await.unwrap();
        assert!(store.stopped_background("a").await.unwrap().is_empty());
        assert!(store.stopped_background("zz").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_running_session_shows_its_background_tasks_and_their_end_is_news() {
        use pdt_core::activity::Activity;
        use pdt_core::conversation::BackgroundTask;
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        let task = BackgroundTask { id: "t1".into(), description: "cargo test".into(), kind: "local_bash".into() };
        // The turn ends (idle, seen) while a task runs.
        store.update_activity("a", Some(Activity::Idle), Some(50)).await.unwrap();
        store.mark_viewed("a", 60).await.unwrap();
        store.set_session_background("a", std::slice::from_ref(&task), 70).await.unwrap();
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.background, s.attention_at, s.unseen), (vec![task.clone()], Some(50), false));
        // The task ends with the agent still idle: news now.
        store.set_session_background("a", &[], 80).await.unwrap();
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.background, s.attention_at, s.unseen), (vec![], Some(80), true));
        // Clearing again changes nothing; nor does a task ending while the agent works.
        store.set_session_background("a", &[], 90).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap().unwrap().attention_at, Some(80));
        store.update_activity("a", Some(Activity::Working), None).await.unwrap();
        store.set_session_background("a", std::slice::from_ref(&task), 100).await.unwrap();
        store.set_session_background("a", &[], 110).await.unwrap();
        assert_eq!(store.get_session("a").await.unwrap().unwrap().attention_at, Some(80));
        // An ended session shows none.
        store.set_session_background("a", std::slice::from_ref(&task), 120).await.unwrap();
        store.mark_running_interrupted(130).await.unwrap();
        assert!(store.get_session("a").await.unwrap().unwrap().background.is_empty());
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
    async fn fresh_start_time_is_recorded_separately_from_running() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        store.end_session("a", SessionStatus::Finished, Some(0), 5).await.unwrap();
        assert!(store.set_session_running("a").await.unwrap());
        let s = store.get_session("a").await.unwrap().unwrap();
        assert_eq!((s.status, s.fresh_started_at, s.ended_at), (SessionStatus::Running, Some(1), None));
        assert!(store.set_fresh_started_at("a", 99).await.unwrap());
        assert_eq!(store.get_session("a").await.unwrap().unwrap().fresh_started_at, Some(99));
        assert_eq!(store.list_codex_sessions().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_unique_index_rejects_a_conversation_owned_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_session(&session("a", 1)).await.unwrap();
        let mut b = session("b", 2);
        b.external_session_id = Some("conv".into());
        store.insert_session(&b).await.unwrap();
        let mut c = session("c", 3);
        c.external_session_id = Some("conv".into());
        assert!(store.insert_session(&c).await.is_err());
        assert!(!store.bind_external_session_id("a", "conv").await.unwrap());
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
            missing: false,
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
