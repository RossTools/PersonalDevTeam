mod attachments;
mod documents;
mod error;
mod event_log;
mod events;
mod health;
mod host;
mod inbox;
mod local_forge;
mod migrate;
mod notifications;
mod personas;
mod pm;
mod review;
mod runs;
mod secrets;
mod sessions;
mod tickets;
mod todos;
mod trusted_commands;
mod watches;
mod workflows;
mod workspaces;
mod worktrees;

pub use attachments::AddAttachment;
pub use documents::{AddDocument, DocumentLink, NewDocumentComment};
pub use error::StoreError;
pub use event_log::{EVENT_RETENTION_MS, EventFilter};
pub use events::{NewEvent, RAW_RETENTION_MS};
pub use health::{WRITE_FAILED, WriteFailure, is_disk_full, is_write_failure, write_backoff_ms};
pub use host::ProjectSettings;
pub use local_forge::{LocalChange, LocalCiRun};
pub use notifications::NotificationFilter;
pub use pm::{NoteFilter, PmSessionRow};
pub use tickets::{
    AttemptInfo, DeleteTicket, Made, OverlapRow, PartRow, StartedAttempt, StoredExternalLink, TicketError,
};
pub use todos::TodoFilter;
pub use workflows::{RUN_RETENTION_MS, RunFilter, SessionMessage};
pub use workspaces::DeleteWorkspace;
pub use worktrees::WorktreeTicketRow;

use pdt_core::project::Project;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous};
use std::path::Path;

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
    /// What `get_host_settings` returns while none are stored (the profile's defaults; see `with_host_defaults`).
    host_defaults: std::sync::Arc<pdt_core::host::HostSettings>,
    /// Known secrets' values, replaced in the text the store keeps (transcripts, comments, steps, runs, the Inbox):
    /// PDT-158. Shared with pdt-api and the log writer (see `with_redactor`).
    redactor: std::sync::Arc<pdt_core::secret::Redactor>,
    /// Writes failing, and the events waiting to be logged once they work (PDT-323).
    health: std::sync::Arc<health::WriteHealth>,
}

pub(crate) type ProjectRow = (String, String, String, String, i64, Option<String>, String);

pub(crate) fn to_project((id, name, repo_path, base_branch, created_at, key, kind): ProjectRow) -> Project {
    Project {
        id,
        name,
        repo_path,
        base_branch,
        created_at,
        key: key.unwrap_or_default(),
        forge: None,
        kind: pdt_core::project::ProjectKind::parse(&kind).unwrap_or_default(),
    }
}

pub(crate) const PROJECT_COLUMNS: &str = "id, name, repo_path, base_branch, created_at, key, kind";

impl Store {
    /// A write transaction: `BEGIN IMMEDIATE`, so it takes the write lock up front. A deferred one that reads first
    /// and then writes fails at once with "database is locked" (SQLITE_BUSY_SNAPSHOT, which the busy timeout does not
    /// retry) when another connection wrote in between, e.g. a new conversation's first events while Start opens its
    /// tabs.
    pub(crate) async fn write_tx(&self) -> Result<sqlx::Transaction<'static, sqlx::Sqlite>, sqlx::Error> {
        // One that can't start (a full disk, a lock held too long) is a failed write, whoever asked (PDT-323).
        self.pool.begin_with("BEGIN IMMEDIATE").await.inspect_err(|e| {
            if health::is_write_error(e) {
                self.note_write_failure(&format!("{WRITE_FAILED}: {e}"));
            }
        })
    }

    /// Opens (creating if needed) the database, backing up to `<db dir>/backups`
    /// before applying any pending migrations.
    pub async fn open(db_path: &Path) -> Result<Store, StoreError> {
        let backup_dir = db_path.parent().unwrap_or(Path::new(".")).join("backups");
        let store = Self::open_with(db_path, &migrate::MIGRATOR, &backup_dir).await?;
        store.backfill_project_keys().await?;
        store.backfill_permission_modes().await?;
        store.backfill_enabled_integrations().await?;
        store.release_gone_sessions().await?;
        Ok(store)
    }

    pub async fn open_with(db_path: &Path, migrator: &Migrator, backup_dir: &Path) -> Result<Store, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            // NORMAL is safe under WAL (a power cut can lose the last commits, never corrupt the file) and saves an
            // fsync per commit.
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true);
        // No ping before handing out an idle connection: a local SQLite connection doesn't go stale, and on Windows
        // that ping could stall a checkout indefinitely under load (with idle connections in the pool), hanging the
        // request that needed it.
        // A connection is never handed out, or kept, inside a transaction (PDT-323): `health::release_clean`.
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .test_before_acquire(false)
            .after_release(|conn, _| Box::pin(health::release_clean(conn)))
            .before_acquire(|conn, _| Box::pin(async move { Ok(!sqlx::Connection::is_in_transaction(conn)) }))
            .connect_with(options)
            .await?;
        migrate::backup_if_pending(&pool, migrator, backup_dir).await?;
        migrator.run(&pool).await?;
        Ok(Store { pool, host_defaults: Default::default(), redactor: Default::default(), health: Default::default() })
    }

    /// Copies the WAL into the database and empties it (`wal_checkpoint(TRUNCATE)`). SQLite's own checkpoints can
    /// keep failing to reset the WAL while readers come and go, so pdtd runs this now and then. Returns whether it
    /// completed (false when a reader or writer was in the way; the next one tries again).
    pub async fn checkpoint(&self) -> Result<bool, StoreError> {
        let (busy, _, _): (i64, i64, i64) =
            sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)").fetch_one(&self.pool).await?;
        Ok(busy == 0)
    }

    /// Host settings to report until some are stored (e.g. `HostSettings::for_profile`).
    pub fn with_host_defaults(mut self, defaults: pdt_core::host::HostSettings) -> Store {
        self.host_defaults = std::sync::Arc::new(defaults);
        self
    }

    /// Uses `r` to redact what it stores (pdtd shares one with its log writer).
    pub fn with_redactor(mut self, r: std::sync::Arc<pdt_core::secret::Redactor>) -> Store {
        self.redactor = r;
        self
    }

    /// The redactor of known secrets' values.
    pub fn redactor(&self) -> &std::sync::Arc<pdt_core::secret::Redactor> {
        &self.redactor
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    /// Gives every project without a key one suggested from its name (projects made before keys existed).
    async fn backfill_project_keys(&self) -> Result<(), StoreError> {
        let missing: Vec<(String, String)> =
            sqlx::query_as("SELECT id, name FROM projects WHERE key IS NULL OR key = '' ORDER BY created_at, id")
                .fetch_all(&self.pool)
                .await?;
        for (id, name) in missing {
            let key = pdt_core::project::suggest_key(&name, &self.project_keys().await?);
            sqlx::query("UPDATE projects SET key = ? WHERE id = ?").bind(key).bind(id).execute(&self.pool).await?;
        }
        Ok(())
    }

    /// Every key in use.
    pub async fn project_keys(&self) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT key FROM projects WHERE key IS NOT NULL").fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|(k,)| k).collect())
    }

    /// Adds a project and its own workspace. A blank `key` gets one suggested from the name.
    pub async fn insert_project(&self, p: &Project) -> Result<(), StoreError> {
        let key = if p.key.trim().is_empty() {
            pdt_core::project::suggest_key(&p.name, &self.project_keys().await?)
        } else {
            p.key.clone()
        };
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "INSERT INTO projects (id, name, repo_path, base_branch, created_at, key, kind) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.name)
        .bind(&p.repo_path)
        .bind(&p.base_branch)
        .bind(p.created_at)
        .bind(key)
        .bind(p.kind.as_str())
        .execute(&mut *tx)
        .await?;
        workspaces::insert_project_workspace(&mut tx, &p.id, &p.name, p.created_at).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        // The host project (the PM's, PDT-81) is PDT's own: never listed.
        let sql = format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE kind != 'host' ORDER BY created_at, id");
        let rows: Vec<ProjectRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(to_project).collect())
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<Project>, StoreError> {
        let sql = format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?");
        let row: Option<ProjectRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(to_project))
    }

    /// Renames a project (and its workspace) and/or changes its key or base branch. False if there is no such
    /// project; `Conflict` if the key is taken.
    pub async fn update_project(
        &self,
        id: &str,
        name: Option<&str>,
        key: Option<&str>,
        base_branch: Option<&str>,
    ) -> Result<bool, StoreError> {
        let mut tx = self.write_tx().await?;
        let found: Option<(String,)> =
            sqlx::query_as("SELECT id FROM projects WHERE id = ?").bind(id).fetch_optional(&mut *tx).await?;
        if found.is_none() {
            return Ok(false);
        }
        if let Some(name) = name {
            sqlx::query("UPDATE projects SET name = ? WHERE id = ?").bind(name).bind(id).execute(&mut *tx).await?;
            sqlx::query("UPDATE workspaces SET name = ? WHERE project_id = ?")
                .bind(name)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        if let Some(key) = key {
            sqlx::query("UPDATE projects SET key = ? WHERE id = ?").bind(key).bind(id).execute(&mut *tx).await?;
        }
        if let Some(base) = base_branch {
            sqlx::query("UPDATE projects SET base_branch = ? WHERE id = ?")
                .bind(base)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Removes a project and everything recorded about it (worktrees, sessions, runs, its workspace and tabs).
    /// Files on disk are not touched.
    pub async fn delete_project(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM projects WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::project::Project;

    fn project(id: &str, path: &str) -> Project {
        Project {
            id: id.into(),
            name: "P".into(),
            repo_path: path.into(),
            base_branch: "main".into(),
            created_at: 1,
            key: String::new(),
            forge: None,
            kind: Default::default(),
        }
    }

    #[tokio::test]
    async fn insert_list_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let mut p = project("a", "C:\\work\\my repo é");
        store.insert_project(&p).await.unwrap();
        p.key = "PX".into();
        assert_eq!(store.list_projects().await.unwrap(), vec![p.clone()]);
        assert_eq!(store.get_project("a").await.unwrap(), Some(p));
        assert_eq!(store.get_project("missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn duplicate_repo_path_is_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        store.insert_project(&project("a", "/r")).await.unwrap();
        let err = store.insert_project(&project("b", "/r")).await.unwrap_err();
        assert!(matches!(err, StoreError::Conflict(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn data_survives_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("pdt.db");
        let store = Store::open(&db).await.unwrap();
        store.insert_project(&project("a", "/r")).await.unwrap();
        store.close().await;
        let store = Store::open(&db).await.unwrap();
        assert_eq!(store.list_projects().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn keys_are_suggested_unique_and_editable() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let mut a = project("a", "/a");
        a.name = "Personal Dev Team".into();
        store.insert_project(&a).await.unwrap();
        let mut b = project("b", "/b");
        b.name = "Pretty Dull Thing".into();
        store.insert_project(&b).await.unwrap();
        let keys: Vec<String> = store.list_projects().await.unwrap().into_iter().map(|p| p.key).collect();
        assert_eq!(keys, ["PDT", "PDT2"]);

        assert!(store.update_project("b", Some("Renamed"), Some("RN"), Some("trunk")).await.unwrap());
        let b = store.get_project("b").await.unwrap().unwrap();
        assert_eq!((b.name.as_str(), b.key.as_str(), b.base_branch.as_str()), ("Renamed", "RN", "trunk"));
        let ws = store.list_workspaces().await.unwrap();
        assert!(ws.iter().any(|w| w.name == "Renamed" && w.project_id.as_deref() == Some("b")));
        assert!(matches!(store.update_project("b", None, Some("PDT"), None).await, Err(StoreError::Conflict(_))));
        assert!(!store.update_project("nope", Some("x"), None, None).await.unwrap());

        assert!(store.delete_project("b").await.unwrap());
        assert!(!store.delete_project("b").await.unwrap());
        assert_eq!(store.list_workspaces().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn existing_projects_get_keys_when_the_store_opens() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("pdt.db");
        let store = Store::open(&db).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, repo_path, base_branch, created_at) VALUES ('x', 'PDT configuration', '/x', 'main', 1), ('y', 'Pdt Config', '/y', 'main', 2)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.close().await;
        let store = Store::open(&db).await.unwrap();
        let keys: Vec<String> = store.list_projects().await.unwrap().into_iter().map(|p| p.key).collect();
        assert_eq!(keys, ["PC", "PC2"]);
    }
}
