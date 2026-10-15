use crate::{Store, StoreError};
use pdt_core::worktree::Worktree;

type WorktreeRow = (String, String, String, String, String, i64);

fn to_worktree((id, project_id, branch, path, base_commit, created_at): WorktreeRow) -> Worktree {
    Worktree { id, project_id, branch, path, base_commit, created_at }
}

const COLUMNS: &str = "id, project_id, branch, path, base_commit, created_at";

impl Store {
    pub async fn insert_worktree(&self, w: &Worktree) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO worktrees (id, project_id, branch, path, base_commit, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&w.id)
        .bind(&w.project_id)
        .bind(&w.branch)
        .bind(&w.path)
        .bind(&w.base_commit)
        .bind(w.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_worktrees(&self, project_id: &str) -> Result<Vec<Worktree>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM worktrees WHERE project_id = ? ORDER BY created_at, id");
        let rows: Vec<WorktreeRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(to_worktree).collect())
    }

    pub async fn get_worktree(&self, id: &str) -> Result<Option<Worktree>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM worktrees WHERE id = ?");
        let row: Option<WorktreeRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(to_worktree))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::{Store, StoreError};
    use pdt_core::project::Project;
    use pdt_core::worktree::Worktree;

    pub(crate) async fn store_with_project(dir: &std::path::Path) -> Store {
        let store = Store::open(&dir.join("pdt.db")).await.unwrap();
        let p = Project {
            id: "p".into(),
            name: "P".into(),
            repo_path: "/r".into(),
            base_branch: "main".into(),
            created_at: 1,
        };
        store.insert_project(&p).await.unwrap();
        store
    }

    fn wt(id: &str, branch: &str) -> Worktree {
        Worktree {
            id: id.into(),
            project_id: "p".into(),
            branch: branch.into(),
            path: format!("/r/.worktrees/{branch}"),
            base_commit: "abc".into(),
            created_at: 2,
        }
    }

    #[tokio::test]
    async fn worktree_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_worktree(&wt("w1", "a")).await.unwrap();
        assert_eq!(store.list_worktrees("p").await.unwrap(), vec![wt("w1", "a")]);
        assert_eq!(store.get_worktree("w1").await.unwrap(), Some(wt("w1", "a")));
        assert_eq!(store.get_worktree("nope").await.unwrap(), None);
        assert!(store.list_worktrees("other").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_branch_is_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_worktree(&wt("w1", "a")).await.unwrap();
        let mut dup = wt("w2", "a");
        dup.path = "/elsewhere".into();
        assert!(matches!(store.insert_worktree(&dup).await, Err(StoreError::Conflict(_))));
    }
}
