//! Run instances.

use crate::{Store, StoreError};
use pdt_core::run::{Run, RunKind, RunState};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const COLUMNS: &str = "id, project_id, worktree_id, branch, name, kind, state, port, url, data_dir, cwd, exit_code, \
                       error, detached, process, created_at, updated_at";
const ACTIVE: &str = "('setting_up', 'starting', 'ready', 'running')";

fn to_run(r: &SqliteRow) -> Result<Run, StoreError> {
    let kind: String = r.try_get("kind")?;
    let state: String = r.try_get("state")?;
    let port: Option<i64> = r.try_get("port")?;
    Ok(Run {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        worktree_id: r.try_get("worktree_id")?,
        branch: r.try_get("branch")?,
        name: r.try_get("name")?,
        kind: RunKind::parse(&kind).ok_or_else(|| StoreError::Corrupt(format!("run kind {kind:?}")))?,
        state: RunState::parse(&state).ok_or_else(|| StoreError::Corrupt(format!("run state {state:?}")))?,
        port: port.and_then(|p| u16::try_from(p).ok()),
        url: r.try_get("url")?,
        data_dir: r.try_get("data_dir")?,
        cwd: r.try_get("cwd")?,
        exit_code: r.try_get("exit_code")?,
        error: r.try_get("error")?,
        detached: r.try_get("detached")?,
        process: r.try_get("process")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

impl Store {
    pub async fn insert_run(&self, r: &Run) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO runs (id, project_id, worktree_id, branch, name, kind, state, port, url, data_dir, cwd, \
             exit_code, error, detached, process, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.id)
        .bind(&r.project_id)
        .bind(&r.worktree_id)
        .bind(&r.branch)
        .bind(&r.name)
        .bind(r.kind.as_str())
        .bind(r.state.as_str())
        .bind(r.port.map(i64::from))
        .bind(&r.url)
        .bind(&r.data_dir)
        .bind(&r.cwd)
        .bind(r.exit_code)
        .bind(&r.error)
        .bind(r.detached)
        .bind(&r.process)
        .bind(r.created_at)
        .bind(r.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_run(&self, id: &str) -> Result<Option<Run>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM runs WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_run).transpose()
    }

    pub async fn list_runs(&self, project_id: &str) -> Result<Vec<Run>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM runs WHERE project_id = ? ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_run).collect()
    }

    /// Every active run on this host, oldest first.
    pub async fn list_active_runs(&self) -> Result<Vec<Run>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM runs WHERE state IN {ACTIVE} ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_run).collect()
    }

    /// Moves to another state, clearing the previous exit code and error.
    pub async fn set_run_state(&self, id: &str, state: RunState, at: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE runs SET state = ?, exit_code = NULL, error = NULL, updated_at = ? WHERE id = ?")
            .bind(state.as_str())
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Ends the run in `state` and forgets its detached processes (they have exited or been ended).
    pub async fn finish_run(
        &self,
        id: &str,
        state: RunState,
        exit_code: Option<i32>,
        error: Option<&str>,
        at: i64,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE runs SET state = ?, exit_code = ?, error = ?, process = NULL, updated_at = ? WHERE id = ?",
        )
        .bind(state.as_str())
        .bind(exit_code)
        .bind(error)
        .bind(at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn set_run_port(&self, id: &str, port: Option<u16>, url: Option<&str>) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE runs SET port = ?, url = ? WHERE id = ?")
            .bind(port.map(i64::from))
            .bind(url)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Whether the run's command is detached, and how to find its processes (`None` once they are gone).
    pub async fn set_run_process(&self, id: &str, detached: bool, process: Option<&str>) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE runs SET detached = ?, process = ? WHERE id = ?")
            .bind(detached)
            .bind(process)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_run(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM runs WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use crate::worktrees::tests::store_with_project;
    use pdt_core::run::{Run, RunKind, RunState};

    pub(crate) fn run(id: &str, port: Option<u16>, at: i64) -> Run {
        Run {
            id: id.into(),
            project_id: "p".into(),
            worktree_id: None,
            branch: "main".into(),
            name: "web".into(),
            kind: RunKind::Run,
            state: RunState::SettingUp,
            port,
            url: port.map(|p| format!("http://127.0.0.1:{p}/")),
            data_dir: Some("/d".into()),
            cwd: "/r".into(),
            exit_code: None,
            error: None,
            detached: false,
            process: None,
            created_at: at,
            updated_at: at,
        }
    }

    #[tokio::test]
    async fn runs_roundtrip_and_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_run(&run("b", Some(8401), 2)).await.unwrap();
        store.insert_run(&run("a", None, 1)).await.unwrap();
        let ids: Vec<_> = store.list_runs("p").await.unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(store.get_run("b").await.unwrap(), Some(run("b", Some(8401), 2)));
        assert!(store.set_run_state("b", RunState::Ready, 5).await.unwrap());
        assert!(store.finish_run("b", RunState::Failed, Some(3), Some("boom"), 9).await.unwrap());
        let b = store.get_run("b").await.unwrap().unwrap();
        assert_eq!(
            (b.state, b.exit_code, b.error.as_deref(), b.updated_at),
            (RunState::Failed, Some(3), Some("boom"), 9)
        );
        assert_eq!(store.list_active_runs().await.unwrap().len(), 1, "only a is active");
        store.set_run_port("a", Some(8402), Some("http://127.0.0.1:8402/")).await.unwrap();
        assert_eq!(store.get_run("a").await.unwrap().unwrap().port, Some(8402));
        assert!(store.delete_run("a").await.unwrap());
        assert!(!store.delete_run("a").await.unwrap());
    }

    #[tokio::test]
    async fn detached_process_is_kept_until_the_run_finishes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_run(&run("a", None, 1)).await.unwrap();
        assert!(store.set_run_process("a", true, Some("pgid:42")).await.unwrap());
        let a = store.get_run("a").await.unwrap().unwrap();
        assert_eq!((a.detached, a.process.as_deref()), (true, Some("pgid:42")));
        assert_eq!(store.list_active_runs().await.unwrap()[0].process.as_deref(), Some("pgid:42"));
        store.finish_run("a", RunState::Stopped, None, None, 2).await.unwrap();
        let a = store.get_run("a").await.unwrap().unwrap();
        assert_eq!((a.detached, a.process), (true, None), "finishing forgets the processes, not the mode");
    }

    #[tokio::test]
    async fn active_runs_cannot_share_a_port() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_run(&run("a", Some(8401), 1)).await.unwrap();
        assert!(store.insert_run(&run("b", Some(8401), 2)).await.is_err());
        store.finish_run("a", RunState::Stopped, None, None, 3).await.unwrap();
        store.insert_run(&run("b", Some(8401), 2)).await.unwrap();
    }
}
