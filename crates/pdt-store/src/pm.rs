//! The PM (PDT-81): the host project its sessions live in, its notes and journal (FTS5), and its sessions' rotation
//! and daily updates.

use crate::{PROJECT_COLUMNS, ProjectRow, Store, StoreError, to_project};
use pdt_core::pm::{NoteKind, PmNote, RotateReason, Rotation};
use pdt_core::project::{HOST_PROJECT_KEY, Project, ProjectKind};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite};

const PM_SESSION_COLUMNS: &str = "session_id, rotation, reason, created_at, ended_at, COALESCE(caught_up_at, created_at) AS caught_up_at, via, \
     reset_from, listed_at";

const NOTE_COLUMNS: &str = "n.id, n.project_id, n.kind, n.title, n.body, n.pinned, n.session_id, n.created_at, \
                            n.updated_at";

fn to_note(r: &SqliteRow) -> Result<PmNote, StoreError> {
    let kind: String = r.try_get("kind")?;
    Ok(PmNote {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        kind: NoteKind::parse(&kind).ok_or_else(|| StoreError::Corrupt(format!("note kind {kind:?}")))?,
        title: r.try_get("title")?,
        body: r.try_get("body")?,
        pinned: r.try_get::<i64, _>("pinned")? != 0,
        session_id: r.try_get("session_id")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

/// What `search_notes` filters on.
#[derive(Debug, Clone, Default)]
pub struct NoteFilter {
    /// Words (FTS5); every note when None.
    pub q: Option<String>,
    /// This project's notes and the host-wide ones; all when None.
    pub project_id: Option<String>,
    pub kind: Option<NoteKind>,
    pub pinned: Option<bool>,
    pub limit: i64,
}

/// A PM session's rotation row.
#[derive(Debug, Clone, PartialEq)]
pub struct PmSessionRow {
    pub session_id: String,
    pub rotation: Rotation,
    pub reason: Option<RotateReason>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    /// When it was last brought up to date: its briefing (`created_at`) or its last daily update (PDT-167).
    pub caught_up_at: i64,
    /// Where it is held besides PDT (PDT-304: `discord`).
    pub via: Option<String>,
    /// The conversation it was reset from (PDT-308).
    pub reset_from: Option<String>,
    /// Where it sits in the list (PDT-308): a reset one's until it has activity of its own (the last activity of the one
    /// it was reset from), and one being reset's while it writes its journal.
    pub listed_at: Option<i64>,
}

fn to_pm_session(r: &SqliteRow) -> Result<PmSessionRow, StoreError> {
    let rotation: String = r.try_get("rotation")?;
    let reason: Option<String> = r.try_get("reason")?;
    Ok(PmSessionRow {
        session_id: r.try_get("session_id")?,
        rotation: Rotation::parse(&rotation).ok_or_else(|| StoreError::Corrupt(format!("rotation {rotation:?}")))?,
        reason: reason.as_deref().and_then(RotateReason::parse),
        created_at: r.try_get("created_at")?,
        ended_at: r.try_get("ended_at")?,
        caught_up_at: r.try_get("caught_up_at")?,
        via: r.try_get("via")?,
        reset_from: r.try_get("reset_from")?,
        listed_at: r.try_get("listed_at")?,
    })
}

impl Store {
    /// The host project (PDT's own, for the PM's sessions), if it was made.
    pub async fn host_project(&self) -> Result<Option<Project>, StoreError> {
        let sql = format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE kind = 'host' ORDER BY created_at LIMIT 1");
        let row: Option<ProjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_optional(&self.pool).await?;
        Ok(row.map(to_project))
    }

    /// The host project, made in `folder` when there is none (no workspace: its sessions have no tabs).
    pub async fn ensure_host_project(&self, id: &str, folder: &str, now: i64) -> Result<Project, StoreError> {
        if let Some(p) = self.host_project().await? {
            return Ok(p);
        }
        sqlx::query(
            "INSERT INTO projects (id, name, repo_path, base_branch, created_at, key, kind) \
             VALUES (?, ?, ?, '', ?, ?, ?)",
        )
        .bind(id)
        .bind(pdt_core::brand::SHORT_NAME)
        .bind(folder)
        .bind(now)
        .bind(HOST_PROJECT_KEY)
        .bind(ProjectKind::Host.as_str())
        .execute(&self.pool)
        .await?;
        self.host_project().await?.ok_or_else(|| StoreError::Corrupt("the host project was not made".into()))
    }

    pub async fn insert_note(&self, n: &PmNote) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO pm_notes (id, project_id, kind, title, body, pinned, session_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&n.id)
        .bind(&n.project_id)
        .bind(n.kind.as_str())
        .bind(&n.title)
        .bind(&n.body)
        .bind(n.pinned)
        .bind(&n.session_id)
        .bind(n.created_at)
        .bind(n.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_note(&self, id: &str) -> Result<Option<PmNote>, StoreError> {
        let sql = format!("SELECT {NOTE_COLUMNS} FROM pm_notes n WHERE n.id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_note).transpose()
    }

    /// Replaces a note's project, title, body and pinned flag (the caller merged the patch). False if it is gone.
    pub async fn update_note(&self, n: &PmNote) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE pm_notes SET project_id = ?, title = ?, body = ?, pinned = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&n.project_id)
        .bind(&n.title)
        .bind(&n.body)
        .bind(n.pinned)
        .bind(n.updated_at)
        .bind(&n.id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_note(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM pm_notes WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Notes matching the filter: by relevance when searching (pinned first otherwise), then newest first.
    pub async fn search_notes(&self, f: &NoteFilter) -> Result<Vec<PmNote>, StoreError> {
        let fts = f.q.as_deref().and_then(pdt_core::pm::fts_query);
        let mut qb = QueryBuilder::<Sqlite>::new(format!("SELECT {NOTE_COLUMNS} FROM pm_notes n"));
        if fts.is_some() {
            qb.push(" JOIN pm_notes_fts f ON f.rowid = n.rowid");
        }
        qb.push(" WHERE 1 = 1");
        if let Some(q) = &fts {
            qb.push(" AND pm_notes_fts MATCH ").push_bind(q.clone());
        }
        if let Some(p) = &f.project_id {
            qb.push(" AND (n.project_id IS NULL OR n.project_id = ").push_bind(p.clone()).push(")");
        }
        if let Some(k) = f.kind {
            qb.push(" AND n.kind = ").push_bind(k.as_str());
        }
        if let Some(pinned) = f.pinned {
            qb.push(" AND n.pinned = ").push_bind(pinned);
        }
        qb.push(if fts.is_some() {
            " ORDER BY f.rank, n.updated_at DESC"
        } else {
            " ORDER BY n.pinned DESC, n.updated_at DESC"
        });
        qb.push(" LIMIT ").push_bind(f.limit.clamp(1, 500));
        let rows = qb.build().fetch_all(&self.pool).await?;
        rows.iter().map(to_note).collect()
    }

    /// A PM session's journal entry.
    pub async fn session_journal(&self, session_id: &str) -> Result<Option<PmNote>, StoreError> {
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM pm_notes n WHERE n.kind = 'journal' AND n.session_id = ? \
             ORDER BY n.created_at DESC LIMIT 1"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_note).transpose()
    }

    pub async fn insert_pm_session(&self, session_id: &str, now: i64) -> Result<(), StoreError> {
        sqlx::query("INSERT OR IGNORE INTO pm_sessions (session_id, created_at) VALUES (?, ?)")
            .bind(session_id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_pm_session(&self, session_id: &str) -> Result<Option<PmSessionRow>, StoreError> {
        let sql = format!("SELECT {PM_SESSION_COLUMNS} FROM pm_sessions WHERE session_id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_pm_session).transpose()
    }

    /// Records where a PM session is held besides PDT (PDT-304: `discord`); None: only in PDT.
    pub async fn set_pm_via(&self, session_id: &str, via: Option<&str>) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE pm_sessions SET via = ? WHERE session_id = ?")
            .bind(via)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// A PM session that is the reset of `from` (PDT-308), listed at `listed_at` until it has activity of its own.
    pub async fn insert_pm_reset(
        &self,
        session_id: &str,
        now: i64,
        from: &str,
        listed_at: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT OR IGNORE INTO pm_sessions (session_id, created_at, reset_from, listed_at) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(now)
        .bind(from)
        .bind(listed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Keeps a PM session the user is resetting where it is in the list (`listed_at`, its last activity before the
    /// reset) while it writes its journal (PDT-308).
    pub async fn set_pm_listed_at(&self, session_id: &str, at: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE pm_sessions SET listed_at = ? WHERE session_id = ?")
            .bind(at)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Every PM session, newest first.
    pub async fn list_pm_sessions(&self) -> Result<Vec<PmSessionRow>, StoreError> {
        let sql = format!("SELECT {PM_SESSION_COLUMNS} FROM pm_sessions ORDER BY created_at DESC, session_id DESC");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_pm_session).collect()
    }

    /// Marks an open PM session brought up to date at `to` if it was last at `from` (false if it ended, or another
    /// caller moved it first).
    pub async fn set_pm_caught_up(&self, session_id: &str, from: i64, to: i64) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE pm_sessions SET caught_up_at = ? \
             WHERE session_id = ? AND rotation = 'none' AND COALESCE(caught_up_at, created_at) = ?",
        )
        .bind(to)
        .bind(session_id)
        .bind(from)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Moves a PM session's rotation on from `from` (false if it was not there: someone else moved it first).
    pub async fn set_pm_rotation(
        &self,
        session_id: &str,
        from: Rotation,
        to: Rotation,
        reason: Option<RotateReason>,
        now: i64,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE pm_sessions SET rotation = ?, reason = COALESCE(?, reason), \
             ended_at = CASE WHEN ? = 'done' THEN ? ELSE ended_at END WHERE session_id = ? AND rotation = ?",
        )
        .bind(to.as_str())
        .bind(reason.map(RotateReason::as_str))
        .bind(to.as_str())
        .bind(now)
        .bind(session_id)
        .bind(from.as_str())
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;

    fn note(id: &str, project: Option<&str>, title: &str, body: &str, pinned: bool, at: i64) -> PmNote {
        PmNote {
            id: id.into(),
            project_id: project.map(String::from),
            kind: NoteKind::Note,
            title: title.into(),
            body: body.into(),
            pinned,
            session_id: None,
            created_at: at,
            updated_at: at,
        }
    }

    #[tokio::test]
    async fn the_host_project_is_made_once_and_never_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        assert_eq!(store.host_project().await.unwrap(), None);
        let h = store.ensure_host_project("h", "/data/pm", 5).await.unwrap();
        assert_eq!((h.id.as_str(), h.kind, h.key.as_str()), ("h", ProjectKind::Host, HOST_PROJECT_KEY));
        assert_eq!(store.ensure_host_project("h2", "/x", 6).await.unwrap().id, "h");
        let listed: Vec<String> = store.list_projects().await.unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(listed, ["p"]);
        assert_eq!(store.get_project("h").await.unwrap().map(|p| p.kind), Some(ProjectKind::Host));
    }

    #[tokio::test]
    async fn notes_are_searched_with_fts_and_filtered() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_note(&note("a", None, "Work hours", "The user works 9 to 5 on weekdays.", true, 1)).await.unwrap();
        store.insert_note(&note("b", Some("p"), "Deploys", "Deploy keys live in the vault.", false, 2)).await.unwrap();
        let ids = |v: Vec<PmNote>| v.into_iter().map(|n| n.id).collect::<Vec<_>>();
        let all = NoteFilter { limit: 50, ..Default::default() };
        assert_eq!(ids(store.search_notes(&all).await.unwrap()), ["a", "b"], "pinned first");
        let q = |q: &str| NoteFilter { q: Some(q.into()), limit: 50, ..Default::default() };
        assert_eq!(ids(store.search_notes(&q("deploy")).await.unwrap()), ["b"]);
        assert_eq!(ids(store.search_notes(&q("week")).await.unwrap()), ["a"], "prefix match");
        assert!(store.search_notes(&q("weekdays vault")).await.unwrap().is_empty(), "every word");
        let other = NoteFilter { project_id: Some("other".into()), limit: 50, ..Default::default() };
        assert_eq!(ids(store.search_notes(&other).await.unwrap()), ["a"], "host-wide notes go everywhere");

        let mut b = store.get_note("b").await.unwrap().unwrap();
        b.body = "Secrets are in 1Password.".into();
        b.updated_at = 3;
        assert!(store.update_note(&b).await.unwrap());
        assert!(store.search_notes(&q("vault")).await.unwrap().is_empty(), "the index follows updates");
        assert_eq!(ids(store.search_notes(&q("1password")).await.unwrap()), ["b"]);
        assert!(store.delete_note("b").await.unwrap());
        assert!(store.search_notes(&q("1password")).await.unwrap().is_empty());
        assert!(!store.delete_note("b").await.unwrap());
    }

    #[tokio::test]
    async fn pm_sessions_rotate_once() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let h = store.ensure_host_project("h", "/data/pm", 5).await.unwrap();
        let s = pdt_core::session::Session { project_id: h.id.clone(), ..crate::sessions::tests::session("s1", 10) };
        store.insert_session(&s).await.unwrap();
        store.insert_pm_session("s1", 10).await.unwrap();
        let row = store.get_pm_session("s1").await.unwrap().unwrap();
        assert_eq!((row.rotation, row.reason, row.ended_at, row.caught_up_at), (Rotation::None, None, None, 10));
        assert_eq!((row.via, row.reset_from, row.listed_at), (None, None, None));
        // Held in Discord's DMs too (PDT-304).
        assert!(store.set_pm_via("s1", Some("discord")).await.unwrap());
        assert_eq!(store.get_pm_session("s1").await.unwrap().unwrap().via.as_deref(), Some("discord"));
        assert!(!store.set_pm_via("nope", Some("discord")).await.unwrap());
        // Brought up to date once per time it was last at (PDT-167).
        assert!(store.set_pm_caught_up("s1", 10, 11).await.unwrap());
        assert!(!store.set_pm_caught_up("s1", 10, 12).await.unwrap(), "another caller moved it first");
        assert_eq!(store.get_pm_session("s1").await.unwrap().unwrap().caught_up_at, 11);
        assert!(
            store.set_pm_rotation("s1", Rotation::None, Rotation::Journal, Some(RotateReason::User), 11).await.unwrap()
        );
        assert!(!store.set_pm_caught_up("s1", 11, 12).await.unwrap(), "not once it is ending");
        assert!(!store.set_pm_rotation("s1", Rotation::None, Rotation::Journal, None, 12).await.unwrap());
        assert!(store.set_pm_rotation("s1", Rotation::Journal, Rotation::Done, None, 13).await.unwrap());
        let row = store.get_pm_session("s1").await.unwrap().unwrap();
        assert_eq!((row.rotation, row.reason, row.ended_at), (Rotation::Done, Some(RotateReason::User), Some(13)));
        let mut j = note("j", None, "Journal", "We talked.", false, 13);
        j.kind = NoteKind::Journal;
        j.session_id = Some("s1".into());
        store.insert_note(&j).await.unwrap();
        assert_eq!(store.session_journal("s1").await.unwrap().map(|n| n.id).as_deref(), Some("j"));
        assert_eq!(store.list_pm_sessions().await.unwrap().len(), 1);

        // A reset (PDT-308): the new one knows the one it carries on from, and the list position it keeps.
        let s2 = pdt_core::session::Session { project_id: h.id.clone(), ..crate::sessions::tests::session("s2", 20) };
        store.insert_session(&s2).await.unwrap();
        store.insert_pm_reset("s2", 20, "s1", 12).await.unwrap();
        let row = store.get_pm_session("s2").await.unwrap().unwrap();
        assert_eq!((row.reset_from.as_deref(), row.listed_at), (Some("s1"), Some(12)));
        assert!(store.set_pm_listed_at("s1", 12).await.unwrap());
        assert_eq!(store.get_pm_session("s1").await.unwrap().unwrap().listed_at, Some(12));
    }
}
