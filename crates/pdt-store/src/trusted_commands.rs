//! The run configuration commands the user approved, per project, by fingerprint (`pdt_core::command_trust`).

use crate::{Store, StoreError};
use pdt_core::command_trust::LEGACY_MARK;
use std::collections::HashSet;

impl Store {
    /// The fingerprints of the project's approved commands (never the legacy mark).
    pub async fn trusted_commands(&self, project_id: &str) -> Result<HashSet<String>, StoreError> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT fingerprint FROM trusted_commands WHERE project_id = ? AND fingerprint != ?")
                .bind(project_id)
                .bind(LEGACY_MARK)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(f,)| f).collect())
    }

    /// Records the user's approval of these commands.
    pub async fn trust_commands(&self, project_id: &str, fingerprints: &[String], now: i64) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for f in fingerprints.iter().filter(|f| f.as_str() != LEGACY_MARK) {
            sqlx::query(
                "INSERT OR IGNORE INTO trusted_commands (project_id, fingerprint, created_at) VALUES (?, ?, ?)",
            )
            .bind(project_id)
            .bind(f)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Whether the project carries the legacy mark (it was added before trust was kept).
    pub async fn has_legacy_trust(&self, project_id: &str) -> Result<bool, StoreError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM trusted_commands WHERE project_id = ? AND fingerprint = ?")
                .bind(project_id)
                .bind(LEGACY_MARK)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    /// Trusts `fingerprints` (the main checkout's commands) in place of the project's legacy mark, once: false when
    /// the mark was already gone (another caller took it).
    pub async fn replace_legacy_trust(
        &self,
        project_id: &str,
        fingerprints: &[String],
        now: i64,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin().await?;
        let gone = sqlx::query("DELETE FROM trusted_commands WHERE project_id = ? AND fingerprint = ?")
            .bind(project_id)
            .bind(LEGACY_MARK)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if gone == 0 {
            return Ok(false);
        }
        for f in fingerprints.iter().filter(|f| f.as_str() != LEGACY_MARK) {
            sqlx::query(
                "INSERT OR IGNORE INTO trusted_commands (project_id, fingerprint, created_at) VALUES (?, ?, ?)",
            )
            .bind(project_id)
            .bind(f)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::worktrees::tests::store_with_project;

    #[tokio::test]
    async fn approvals_are_kept_per_project_and_go_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        // A project added now has nothing trusted and no legacy mark.
        assert!(store.trusted_commands("p").await.unwrap().is_empty());
        assert!(!store.has_legacy_trust("p").await.unwrap());
        store.trust_commands("p", &["a".into(), "b".into(), "a".into(), "legacy".into()], 1).await.unwrap();
        assert_eq!(store.trusted_commands("p").await.unwrap(), ["a".to_string(), "b".to_string()].into());
        assert!(!store.has_legacy_trust("p").await.unwrap(), "the mark can't be approved in");
        assert!(store.trusted_commands("other").await.unwrap().is_empty());
        assert!(store.delete_project("p").await.unwrap());
        assert!(store.trusted_commands("p").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_legacy_mark_is_replaced_once() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        sqlx::query("INSERT INTO trusted_commands (project_id, fingerprint, created_at) VALUES ('p', 'legacy', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(store.has_legacy_trust("p").await.unwrap());
        assert!(store.trusted_commands("p").await.unwrap().is_empty());
        assert!(store.replace_legacy_trust("p", &["main".into()], 1).await.unwrap());
        assert!(!store.replace_legacy_trust("p", &["other".into()], 2).await.unwrap());
        assert!(!store.has_legacy_trust("p").await.unwrap());
        assert_eq!(store.trusted_commands("p").await.unwrap(), ["main".to_string()].into());
    }
}
