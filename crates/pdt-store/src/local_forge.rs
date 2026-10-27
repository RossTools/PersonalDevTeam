//! The local forge (PDT-178): change requests of projects with no remote (an attempt's branch compared with its base)
//! and the CI runs of their `[ci]` command, kept here because no forge keeps them.

use crate::{Store, StoreError};
use pdt_core::pr::{CiState, PrState};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// A change request on the local forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalChange {
    pub id: i64,
    pub project_id: String,
    /// From 1 in each project.
    pub number: u32,
    pub branch: String,
    pub base_branch: String,
    pub title: String,
    pub state: PrState,
    /// Set to merge once CI passes.
    pub auto_merge: bool,
    /// The branch head last seen while open, or the head merged.
    pub head_sha: String,
    pub created_at: i64,
    pub merged_at: Option<i64>,
}

/// One run of a local change request's CI command on one head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCiRun {
    pub id: i64,
    pub change_id: i64,
    pub head_sha: String,
    pub command: String,
    /// Running, passed, failed or cancelled.
    pub state: CiState,
    pub exit_code: Option<i64>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
}

fn pr_state(s: &str) -> PrState {
    match s {
        "merged" => PrState::Merged,
        "closed" => PrState::Closed,
        _ => PrState::Open,
    }
}

fn pr_state_str(s: PrState) -> &'static str {
    match s {
        PrState::Open => "open",
        PrState::Merged => "merged",
        PrState::Closed => "closed",
    }
}

fn ci_state(s: &str) -> CiState {
    match s {
        "passed" => CiState::Passed,
        "failed" => CiState::Failed,
        "cancelled" => CiState::Cancelled,
        "queued" => CiState::Queued,
        _ => CiState::Running,
    }
}

fn ci_state_str(s: CiState) -> &'static str {
    match s {
        CiState::Passed => "passed",
        CiState::Failed => "failed",
        CiState::Cancelled => "cancelled",
        CiState::Queued => "queued",
        CiState::Running => "running",
    }
}

const CHANGE: &str = "SELECT id, project_id, number, branch, base_branch, title, state, auto_merge, head_sha, \
                      created_at, merged_at FROM local_changes";
const RUN: &str = "SELECT id, change_id, head_sha, command, state, exit_code, created_at, ended_at FROM local_ci_runs";

fn to_change(r: &SqliteRow) -> Result<LocalChange, StoreError> {
    Ok(LocalChange {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        number: r.try_get::<i64, _>("number")? as u32,
        branch: r.try_get("branch")?,
        base_branch: r.try_get("base_branch")?,
        title: r.try_get("title")?,
        state: pr_state(&r.try_get::<String, _>("state")?),
        auto_merge: r.try_get::<i64, _>("auto_merge")? != 0,
        head_sha: r.try_get("head_sha")?,
        created_at: r.try_get("created_at")?,
        merged_at: r.try_get("merged_at")?,
    })
}

fn to_run(r: &SqliteRow) -> Result<LocalCiRun, StoreError> {
    Ok(LocalCiRun {
        id: r.try_get("id")?,
        change_id: r.try_get("change_id")?,
        head_sha: r.try_get("head_sha")?,
        command: r.try_get("command")?,
        state: ci_state(&r.try_get::<String, _>("state")?),
        exit_code: r.try_get("exit_code")?,
        created_at: r.try_get("created_at")?,
        ended_at: r.try_get("ended_at")?,
    })
}

impl Store {
    /// The change request of `branch` in the project: its open one, else its newest.
    pub async fn local_change_for_branch(
        &self,
        project_id: &str,
        branch: &str,
    ) -> Result<Option<LocalChange>, StoreError> {
        let sql =
            format!("{CHANGE} WHERE project_id = ? AND branch = ? ORDER BY (state = 'open') DESC, id DESC LIMIT 1");
        let row =
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).bind(branch).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_change).transpose()
    }

    /// Change request `number` of the project.
    pub async fn local_change(&self, project_id: &str, number: u32) -> Result<Option<LocalChange>, StoreError> {
        let sql = format!("{CHANGE} WHERE project_id = ? AND number = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(project_id)
            .bind(number as i64)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(to_change).transpose()
    }

    /// Opens a change request of `branch` into `base` at `head`, numbered after the project's last.
    pub async fn open_local_change(
        &self,
        project_id: &str,
        branch: &str,
        base: &str,
        title: &str,
        head: &str,
        now: i64,
    ) -> Result<LocalChange, StoreError> {
        let mut tx = self.write_tx().await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO local_changes (project_id, number, branch, base_branch, title, state, head_sha, created_at) \
             VALUES (?, (SELECT COALESCE(MAX(number), 0) + 1 FROM local_changes WHERE project_id = ?), ?, ?, ?, \
             'open', ?, ?) RETURNING id",
        )
        .bind(project_id)
        .bind(project_id)
        .bind(branch)
        .bind(base)
        .bind(title)
        .bind(head)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?;
        let row =
            sqlx::query(sqlx::AssertSqlSafe(format!("{CHANGE} WHERE id = ?"))).bind(id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        to_change(&row)
    }

    /// Records the open change request's branch head.
    pub async fn set_local_change_head(&self, id: i64, head: &str) -> Result<(), StoreError> {
        sqlx::query("UPDATE local_changes SET head_sha = ? WHERE id = ? AND state = 'open'")
            .bind(head)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Sets the open change request to merge once CI passes (or not).
    pub async fn set_local_change_auto_merge(&self, id: i64, on: bool) -> Result<(), StoreError> {
        sqlx::query("UPDATE local_changes SET auto_merge = ? WHERE id = ? AND state = 'open'")
            .bind(on)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Ends the change request as `state` (merged at `head`, or closed).
    pub async fn end_local_change(&self, id: i64, state: PrState, head: &str, now: i64) -> Result<(), StoreError> {
        let merged_at = (state == PrState::Merged).then_some(now);
        sqlx::query(
            "UPDATE local_changes SET state = ?, head_sha = ?, auto_merge = 0, merged_at = ? WHERE id = ? AND \
             state = 'open'",
        )
        .bind(pr_state_str(state))
        .bind(head)
        .bind(merged_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The change request's CI runs, oldest first.
    pub async fn local_ci_runs(&self, change_id: i64) -> Result<Vec<LocalCiRun>, StoreError> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!("{RUN} WHERE change_id = ? ORDER BY id")))
            .bind(change_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(to_run).collect()
    }

    pub async fn local_ci_run(&self, id: i64) -> Result<Option<LocalCiRun>, StoreError> {
        let row =
            sqlx::query(sqlx::AssertSqlSafe(format!("{RUN} WHERE id = ?"))).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_run).transpose()
    }

    /// Starts a CI run of `command` on `head` (running).
    pub async fn add_local_ci_run(
        &self,
        change_id: i64,
        head: &str,
        command: &str,
        now: i64,
    ) -> Result<LocalCiRun, StoreError> {
        let row = sqlx::query(
            "INSERT INTO local_ci_runs (change_id, head_sha, command, state, created_at) VALUES (?, ?, ?, 'running', \
             ?) RETURNING id, change_id, head_sha, command, state, exit_code, created_at, ended_at",
        )
        .bind(change_id)
        .bind(head)
        .bind(command)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        to_run(&row)
    }

    /// Ends a running CI run as `state`; false when it had already ended.
    pub async fn finish_local_ci_run(
        &self,
        id: i64,
        state: CiState,
        exit_code: Option<i64>,
        now: i64,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE local_ci_runs SET state = ?, exit_code = ?, ended_at = ? WHERE id = ? AND state = 'running'",
        )
        .bind(ci_state_str(state))
        .bind(exit_code)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (Store, tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let repo = std::env::temp_dir().join("pdt-local-forge-store-test");
        let p = pdt_core::project::Project {
            id: "p".into(),
            name: "P".into(),
            repo_path: repo.to_string_lossy().into_owned(),
            base_branch: "main".into(),
            created_at: 1,
            key: String::new(),
            github_repo: None,
            kind: Default::default(),
        };
        s.insert_project(&p).await.unwrap();
        (s, tmp, p.id)
    }

    #[tokio::test]
    async fn change_requests_are_numbered_per_project_and_the_open_one_is_the_branchs() {
        let (s, _tmp, p) = store().await;
        assert_eq!(s.local_change_for_branch(&p, "b").await.unwrap(), None);
        let one = s.open_local_change(&p, "b", "main", "T", "h1", 10).await.unwrap();
        assert_eq!((one.number, one.state, one.auto_merge), (1, PrState::Open, false));
        s.set_local_change_head(one.id, "h2").await.unwrap();
        s.set_local_change_auto_merge(one.id, true).await.unwrap();
        let now = s.local_change_for_branch(&p, "b").await.unwrap().unwrap();
        assert_eq!((now.head_sha.as_str(), now.auto_merge), ("h2", true));
        s.end_local_change(one.id, PrState::Merged, "h2", 20).await.unwrap();
        let merged = s.local_change(&p, 1).await.unwrap().unwrap();
        assert_eq!((merged.state, merged.merged_at, merged.auto_merge), (PrState::Merged, Some(20), false));
        s.set_local_change_head(one.id, "h3").await.unwrap();
        assert_eq!(s.local_change(&p, 1).await.unwrap().unwrap().head_sha, "h2", "a merged one keeps its head");
        let two = s.open_local_change(&p, "b", "main", "T", "h4", 30).await.unwrap();
        assert_eq!(two.number, 2);
        assert_eq!(s.local_change_for_branch(&p, "b").await.unwrap().unwrap().number, 2, "the open one");
    }

    #[tokio::test]
    async fn ci_runs_end_once() {
        let (s, _tmp, p) = store().await;
        let c = s.open_local_change(&p, "b", "main", "T", "h1", 10).await.unwrap();
        let r = s.add_local_ci_run(c.id, "h1", "make", 11).await.unwrap();
        assert_eq!((r.state, r.ended_at), (CiState::Running, None));
        assert!(s.finish_local_ci_run(r.id, CiState::Failed, Some(2), 12).await.unwrap());
        assert!(!s.finish_local_ci_run(r.id, CiState::Passed, Some(0), 13).await.unwrap(), "already ended");
        let runs = s.local_ci_runs(c.id).await.unwrap();
        assert_eq!((runs[0].state, runs[0].exit_code, runs[0].ended_at), (CiState::Failed, Some(2), Some(12)));
        assert_eq!(s.local_ci_run(r.id).await.unwrap().unwrap().command, "make");
    }
}
