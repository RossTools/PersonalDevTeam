mod error;
mod migrate;
mod sessions;
mod worktrees;

pub use error::StoreError;

use pdt_core::project::Project;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::path::Path;

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

type ProjectRow = (String, String, String, String, i64);

fn to_project((id, name, repo_path, base_branch, created_at): ProjectRow) -> Project {
    Project { id, name, repo_path, base_branch, created_at }
}

impl Store {
    /// Opens (creating if needed) the database, backing up to `<db dir>/backups`
    /// before applying any pending migrations.
    pub async fn open(db_path: &Path) -> Result<Store, StoreError> {
        let backup_dir = db_path.parent().unwrap_or(Path::new(".")).join("backups");
        Self::open_with(db_path, &migrate::MIGRATOR, &backup_dir).await
    }

    pub async fn open_with(db_path: &Path, migrator: &Migrator, backup_dir: &Path) -> Result<Store, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new().max_connections(8).connect_with(options).await?;
        migrate::backup_if_pending(&pool, migrator, backup_dir).await?;
        migrator.run(&pool).await?;
        Ok(Store { pool })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    pub async fn insert_project(&self, p: &Project) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO projects (id, name, repo_path, base_branch, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&p.id)
            .bind(&p.name)
            .bind(&p.repo_path)
            .bind(&p.base_branch)
            .bind(p.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        let rows: Vec<ProjectRow> =
            sqlx::query_as("SELECT id, name, repo_path, base_branch, created_at FROM projects ORDER BY created_at, id")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(to_project).collect())
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<Project>, StoreError> {
        let row: Option<ProjectRow> =
            sqlx::query_as("SELECT id, name, repo_path, base_branch, created_at FROM projects WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(to_project))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::project::Project;

    fn project(id: &str, path: &str) -> Project {
        Project { id: id.into(), name: "P".into(), repo_path: path.into(), base_branch: "main".into(), created_at: 1 }
    }

    #[tokio::test]
    async fn insert_list_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let p = project("a", "C:\\work\\my repo é");
        store.insert_project(&p).await.unwrap();
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
}
