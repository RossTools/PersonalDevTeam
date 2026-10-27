use crate::StoreError;
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use std::collections::HashSet;
use std::path::Path;

/// Every migration in crates/pdt-store/migrations, embedded at build time (build.rs rebuilds when the folder changes).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The last numbered migration (`0029_…`). Every later one is named by its UTC creation time, `YYYYMMDDHHMMSS_<what>.sql`,
/// so two branches cut from the same main never pick the same name.
#[cfg(test)]
const LAST_NUMBERED: i64 = 29;

/// What is wrong with a migrations folder's file names: each must be `<version>_<what>.sql`, the version a number up to
/// [`LAST_NUMBERED`] or a UTC timestamp `YYYYMMDDHHMMSS`, and no two files may share a version (sqlx would refuse the
/// second, or apply one and skip the other).
#[cfg(test)]
fn name_problems<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: std::collections::HashMap<i64, &str> = std::collections::HashMap::new();
    for name in names {
        let Some((version, what)) = name.split_once('_').filter(|(_, w)| w.ends_with(".sql") && w.len() > 4) else {
            problems.push(format!("{name}: not <version>_<what>.sql"));
            continue;
        };
        let Some(v) = version.parse::<i64>().ok().filter(|_| version.bytes().all(|b| b.is_ascii_digit())) else {
            problems.push(format!("{name}: the version is not a number"));
            continue;
        };
        let numbered = version.len() == 4 && v <= LAST_NUMBERED;
        let stamped = version.len() == 14 && jiff::civil::DateTime::strptime("%Y%m%d%H%M%S", version).is_ok();
        if !numbered && !stamped {
            problems.push(format!(
                "{name}: name it by its UTC creation time, YYYYMMDDHHMMSS_{what} (`date -u +%Y%m%d%H%M%S`)"
            ));
        }
        if let Some(other) = seen.insert(v, name) {
            problems.push(format!("{name}: version {v} is also {other}'s"));
        }
    }
    problems
}

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
    prune_backups(backup_dir, KEEP_BACKUPS);
    Ok(())
}

/// How many pre-migration backups are kept (the newest).
pub(crate) const KEEP_BACKUPS: usize = 3;

/// Deletes all but the newest `keep` backups (`pdt-<ms>[-<n>].db`); other files are left alone. Never fails.
pub(crate) fn prune_backups(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut backups: Vec<((i64, u32), std::path::PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| backup_order(&e.file_name().to_string_lossy()).map(|k| (k, e.path())))
        .collect();
    backups.sort_by_key(|(k, _)| std::cmp::Reverse(*k));
    for (_, path) in backups.into_iter().skip(keep) {
        let _ = std::fs::remove_file(&path);
    }
}

/// The stamp and counter of a backup's name (`pdt-<ms>.db` is `(ms, 0)`, `pdt-<ms>-<n>.db` is `(ms, n)`).
fn backup_order(name: &str) -> Option<(i64, u32)> {
    let stem = name.strip_prefix("pdt-")?.strip_suffix(".db")?;
    let (ms, n) = stem.split_once('-').unwrap_or((stem, "0"));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(ms) && digits(n)).then(|| Some((ms.parse().ok()?, n.parse().ok()?))).flatten()
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

    /// The CI check: a duplicate or misnamed migration fails `cargo test`.
    #[test]
    fn the_migrations_folder_is_well_named() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        let problems = super::name_problems(names.iter().map(String::as_str));
        assert!(problems.is_empty(), "crates/pdt-store/migrations:\n{}", problems.join("\n"));
        assert_eq!(super::MIGRATOR.iter().count(), names.len(), "sqlx embeds every file");
    }

    #[test]
    fn names_take_old_numbers_or_timestamps() {
        let ok =
            ["0001_projects.sql", "0029_session_ticket.sql", "20261022143000_add_x.sql", "20261022143001_add_y.sql"];
        assert!(super::name_problems(ok).is_empty());
        for bad in [
            "0030_next_number.sql",
            "20261322143000_bad_month.sql",
            "2026102214300_too_short.sql",
            "20261022143000.sql",
            "notes.sql",
            "x1_a.sql",
            "20261022143000_a.txt",
        ] {
            assert_eq!(super::name_problems([bad]).len(), 1, "{bad}");
        }
    }

    #[test]
    fn a_duplicate_version_is_a_problem() {
        let problems =
            super::name_problems(["20261022143000_a.sql", "20261022143000_b.sql", "0005_runs.sql", "05_again.sql"]);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("20261022143000_b.sql") && problems[0].contains("20261022143000_a.sql"));
    }

    /// Two branches cut from the same main each add a migration; whichever merges second carries the older timestamp
    /// or the newer one. Either way a database that already has the other one applies it.
    #[tokio::test]
    async fn a_migration_older_than_the_last_applied_still_applies() {
        let tmp = tempfile::tempdir().unwrap();
        let backup_dir = tmp.path().join("backups");
        let base = [("0001_a.sql", "CREATE TABLE a (x INTEGER);")];
        let one = ("20261022100000_one.sql", "CREATE TABLE one (x INTEGER);");
        let two = ("20261022110000_two.sql", "CREATE TABLE two (x INTEGER);");
        for (first, second) in [(one, two), (two, one)] {
            let db = tmp.path().join(format!("{}.db", first.0));
            let before = tmp.path().join(format!("before-{}", first.0));
            let after = tmp.path().join(format!("after-{}", first.0));
            for (n, sql) in base.iter().chain([&first]) {
                write(&before, n, sql);
            }
            for (n, sql) in base.iter().chain([&first, &second]) {
                write(&after, n, sql);
            }
            let s = Store::open_with(&db, &Migrator::new(before.as_path()).await.unwrap(), &backup_dir).await.unwrap();
            s.close().await;
            let s = Store::open_with(&db, &Migrator::new(after.as_path()).await.unwrap(), &backup_dir).await.unwrap();
            let tables: Vec<(String,)> = sqlx::query_as(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ('a', 'one', 'two') ORDER BY name",
            )
            .fetch_all(&s.pool)
            .await
            .unwrap();
            assert_eq!(tables, [("a".into(),), ("one".into(),), ("two".into(),)], "{} then {}", first.0, second.0);
            s.close().await;
        }
    }

    /// An existing database, migrated by the numbered migrations alone, takes a timestamped one with no manual step.
    #[tokio::test]
    async fn an_existing_database_takes_a_timestamped_migration() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let backup_dir = tmp.path().join("backups");
        let s = Store::open_with(&db, &super::MIGRATOR, &backup_dir).await.unwrap();
        s.close().await;
        let next = tmp.path().join("next");
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for e in std::fs::read_dir(&src).unwrap() {
            let e = e.unwrap();
            std::fs::create_dir_all(&next).unwrap();
            std::fs::copy(e.path(), next.join(e.file_name())).unwrap();
        }
        write(&next, "29991231235959_later.sql", "CREATE TABLE later (x INTEGER);");
        let s = Store::open_with(&db, &Migrator::new(next.as_path()).await.unwrap(), &backup_dir).await.unwrap();
        let later: Option<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'later'")
                .fetch_optional(&s.pool)
                .await
                .unwrap();
        assert!(later.is_some());
        s.close().await;
        assert_eq!(backups(&backup_dir), 1, "backed up before the new migration");
    }

    #[test]
    fn only_the_newest_backups_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let names = ["pdt-9.db", "pdt-10.db", "pdt-10-1.db", "pdt-11.db", "pdt-8.db", "notes.txt", "pdt-x.db"];
        for n in names {
            std::fs::write(tmp.path().join(n), "").unwrap();
        }
        super::prune_backups(tmp.path(), 3);
        let mut left: Vec<String> =
            std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["notes.txt", "pdt-10-1.db", "pdt-10.db", "pdt-11.db", "pdt-x.db"]);
    }

    #[tokio::test]
    async fn a_backup_prunes_the_old_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let backup_dir = tmp.path().join("backups");
        for stamp in 1..=5 {
            write(&backup_dir, &format!("pdt-{stamp}.db"), "");
        }
        let m1 = tmp.path().join("m1");
        write(&m1, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        let m2 = tmp.path().join("m2");
        write(&m2, "0001_a.sql", "CREATE TABLE a (x INTEGER);");
        write(&m2, "0002_b.sql", "CREATE TABLE b (y INTEGER);");
        Store::open_with(&db, &Migrator::new(m1.as_path()).await.unwrap(), &backup_dir).await.unwrap().close().await;
        Store::open_with(&db, &Migrator::new(m2.as_path()).await.unwrap(), &backup_dir).await.unwrap().close().await;
        assert_eq!(backups(&backup_dir), super::KEEP_BACKUPS);
        assert!(!backup_dir.join("pdt-3.db").exists(), "the oldest went");
        assert!(backup_dir.join("pdt-5.db").exists());
    }

    #[tokio::test]
    async fn a_checkpoint_empties_the_wal() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("t.db");
        let s = Store::open(&db).await.unwrap();
        let mode: i64 = sqlx::query_scalar("PRAGMA synchronous").fetch_one(&s.pool).await.unwrap();
        assert_eq!(mode, 1, "NORMAL");
        let wal = tmp.path().join("t.db-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 0, "the migrations went through the WAL");
        assert!(s.checkpoint().await.unwrap());
        assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
        s.close().await;
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

#[cfg(test)]
mod session_title_tests {
    use crate::Store;
    use sqlx::migrate::Migrator;

    /// 0012 strips the agent prefix PDT used to put on session titles, and nothing else.
    #[tokio::test]
    async fn migration_strips_agent_prefixes_from_titles() {
        let tmp = tempfile::tempdir().unwrap();
        let before = tmp.path().join("before");
        std::fs::create_dir_all(&before).unwrap();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for e in std::fs::read_dir(&src).unwrap() {
            let name = e.unwrap().file_name().into_string().unwrap();
            if name.as_str() < "0012" {
                std::fs::copy(src.join(&name), before.join(&name)).unwrap();
            }
        }
        let db = tmp.path().join("t.db");
        let backups = tmp.path().join("backups");
        let s = Store::open_with(&db, &Migrator::new(before.as_path()).await.unwrap(), &backups).await.unwrap();
        sqlx::query("INSERT INTO projects (id, name, repo_path, base_branch, created_at, key) VALUES ('p', 'P', '/r', 'main', 0, 'P')")
            .execute(&s.pool)
            .await
            .unwrap();
        let rows = [
            ("a", "agent_tui", Some("claude"), "Claude · feature-x"),
            ("b", "agent_tui", Some("codex"), "Codex · Fix login"),
            ("c", "shell", None, "Shell · main"),
            ("d", "agent_tui", Some("codex"), "Claude · named by hand"),
            ("e", "agent_tui", Some("claude"), "My own title"),
            ("f", "agent_tui", Some("claude"), "Claude · "),
        ];
        for (id, kind, agent, title) in rows {
            sqlx::query("INSERT INTO sessions (id, project_id, kind, agent, title, cwd, status, created_at) VALUES (?, 'p', ?, ?, ?, '/r', 'finished', 0)")
                .bind(id)
                .bind(kind)
                .bind(agent)
                .bind(title)
                .execute(&s.pool)
                .await
                .unwrap();
        }
        s.close().await;

        let s = Store::open_with(&db, &super::MIGRATOR, &backups).await.unwrap();
        let titles: Vec<(String,)> =
            sqlx::query_as("SELECT title FROM sessions ORDER BY id").fetch_all(&s.pool).await.unwrap();
        let titles: Vec<&str> = titles.iter().map(|(t,)| t.as_str()).collect();
        assert_eq!(titles, ["feature-x", "Fix login", "main", "Claude · named by hand", "My own title", "Claude · "]);
    }
}

#[cfg(test)]
mod integration_settings_tests {
    use crate::Store;
    use serde_json::json;
    use sqlx::migrate::Migrator;

    /// PDT-174: the Jira and Buildkite columns move into the integration settings, read back the same, and the database
    /// is copied before the migration rewrites it.
    #[tokio::test]
    async fn jira_and_buildkite_settings_move_into_the_integration_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let before = tmp.path().join("before");
        std::fs::create_dir_all(&before).unwrap();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for e in std::fs::read_dir(&src).unwrap() {
            let name = e.unwrap().file_name().into_string().unwrap();
            if !name.ends_with("_integration_settings.sql") {
                std::fs::copy(src.join(&name), before.join(&name)).unwrap();
            }
        }
        let db = tmp.path().join("t.db");
        let backups = tmp.path().join("backups");
        let s = Store::open_with(&db, &Migrator::new(before.as_path()).await.unwrap(), &backups).await.unwrap();
        for (id, key) in [("both", "B"), ("jira", "J"), ("none", "N"), ("bad", "X")] {
            sqlx::query("INSERT INTO projects (id, name, repo_path, base_branch, created_at, key) VALUES (?, ?, '/r/' || ?, 'main', 0, ?)")
                .bind(id)
                .bind(id)
                .bind(id)
                .bind(key)
                .execute(&s.pool)
                .await
                .unwrap();
        }
        let rows = [
            ("both", Some("work-jira"), "project = ABC", "Close", Some("bk"), r#"["app","e2e"]"#),
            ("jira", Some("work-jira"), "", "Done", Some(""), "[]"),
            ("none", None, "", "Done", None, "[]"),
            ("bad", None, "", "Done", Some("bk"), "not json"),
        ];
        for (id, jc, scope, done, bc, pipelines) in rows {
            sqlx::query(
                "INSERT INTO project_settings (project_id, jira_connection, jira_scope, jira_done, buildkite_connection, \
                 buildkite_pipelines) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(jc)
            .bind(scope)
            .bind(done)
            .bind(bc)
            .bind(pipelines)
            .execute(&s.pool)
            .await
            .unwrap();
        }
        s.close().await;

        let s = Store::open_with(&db, &super::MIGRATOR, &backups).await.unwrap();
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1, "copied before migrating");
        let both = s.get_project_settings("both").await.unwrap();
        assert_eq!(
            serde_json::to_value(&both.integrations).unwrap(),
            json!({"jira": {"connection": "work-jira", "scope": "project = ABC", "done": "Close"},
                   "buildkite": {"connection": "bk", "pipelines": ["app", "e2e"]}})
        );
        let only = s.get_project_settings("jira").await.unwrap();
        assert_eq!(only.integrations.keys().collect::<Vec<_>>(), ["jira"], "an empty connection is no link");
        assert!(!only.integrations.contains_key("buildkite"));
        assert!(s.get_project_settings("none").await.unwrap().integrations.is_empty());
        let bad = s.get_project_settings("bad").await.unwrap();
        assert_eq!(
            bad.integrations["buildkite"]["pipelines"],
            json!([]),
            "unreadable pipelines become none rather than failing the migration"
        );
        s.close().await;
    }
}
