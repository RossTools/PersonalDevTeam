use crate::StoreError;
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use std::collections::HashSet;
use std::path::Path;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

async fn applied_versions(pool: &SqlitePool) -> Result<HashSet<i64>, StoreError> {
    let table: Option<(String,)> =
        sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'")
            .fetch_optional(pool)
            .await?;
    if table.is_none() {
        return Ok(HashSet::new());
    }
    let rows: Vec<(i64,)> =
        sqlx::query_as("SELECT version FROM _sqlx_migrations WHERE success = 1").fetch_all(pool).await?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
}

/// Copies the database (VACUUM INTO) when it already has applied migrations
/// and the migrator would apply more. Fresh databases are not backed up.
pub async fn backup_if_pending(pool: &SqlitePool, migrator: &Migrator, backup_dir: &Path) -> Result<(), StoreError> {
    let applied = applied_versions(pool).await?;
    let pending = migrator.iter().any(|m| !applied.contains(&m.version));
    if applied.is_empty() || !pending {
        return Ok(());
    }
    std::fs::create_dir_all(backup_dir)?;
    let file = backup_file(backup_dir, jiff::Timestamp::now().as_millisecond());
    sqlx::query("VACUUM INTO ?").bind(file.to_string_lossy().into_owned()).execute(pool).await?;
    Ok(())
}

/// `pdt-<ms>.db`, or `pdt-<ms>-<n>.db` when that name is taken.
pub(crate) fn backup_file(dir: &Path, stamp: i64) -> std::path::PathBuf {
    (0u32..)
        .map(|n| dir.join(if n == 0 { format!("pdt-{stamp}.db") } else { format!("pdt-{stamp}-{n}.db") }))
        .find(|p| !p.exists())
        .expect("an unused backup name exists")
}

#[cfg(test)]
mod tests {
    use crate::Store;
    use sqlx::migrate::Migrator;
    use std::path::Path;

    fn write(dir: &Path, name: &str, sql: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), sql).unwrap();
    }

    fn backups(dir: &Path) -> usize {
        std::fs::read_dir(dir).map(|r| r.count()).unwrap_or(0)
    }

    #[tokio::test]
    async fn fresh_db_makes_no_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let mig = tmp.path().join("m1");
        write(&mig, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        let migrator = Migrator::new(mig.as_path()).await.unwrap();
        let backup_dir = tmp.path().join("backups");
        let s = Store::open_with(&tmp.path().join("t.db"), &migrator, &backup_dir).await.unwrap();
        s.close().await;
        assert_eq!(backups(&backup_dir), 0);
    }

    #[tokio::test]
    async fn backs_up_before_pending_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let backup_dir = tmp.path().join("backups");
        let m1 = tmp.path().join("m1");
        write(&m1, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        let m2 = tmp.path().join("m2");
        write(&m2, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        write(&m2, "0002_b.sql", "CREATE TABLE b (y INTEGER);");

        let s = Store::open_with(&db, &Migrator::new(m1.as_path()).await.unwrap(), &backup_dir).await.unwrap();
        s.close().await;
        assert_eq!(backups(&backup_dir), 0, "no pending migration on reopen with same set");

        let s = Store::open_with(&db, &Migrator::new(m2.as_path()).await.unwrap(), &backup_dir).await.unwrap();
        s.close().await;
        assert_eq!(backups(&backup_dir), 1, "one backup before applying 0002");
    }

    #[test]
    fn backup_names_never_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let first = super::backup_file(tmp.path(), 42);
        assert_eq!(first, tmp.path().join("pdt-42.db"));
        std::fs::write(&first, "").unwrap();
        let second = super::backup_file(tmp.path(), 42);
        assert_eq!(second, tmp.path().join("pdt-42-1.db"));
    }
}
