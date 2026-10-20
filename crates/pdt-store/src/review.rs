//! Review comments (Phase 6, review spec §3): line comments on a worktree's changes, deleted with the worktree.

use crate::{Store, StoreError};
use pdt_core::review::{CommentSide, ReviewComment};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const COLUMNS: &str = "id, worktree_id, parent_id, path, side, line, end_line, line_text, body, author, sent_at, \
                       session_id, resolved_at, created_at, updated_at";

fn to_comment(r: &SqliteRow) -> Result<ReviewComment, StoreError> {
    let side: String = r.try_get("side")?;
    let n = |c: &str| -> Result<u32, StoreError> { Ok(u32::try_from(r.try_get::<i64, _>(c)?).unwrap_or(0)) };
    Ok(ReviewComment {
        id: r.try_get("id")?,
        worktree_id: r.try_get("worktree_id")?,
        parent_id: r.try_get("parent_id")?,
        path: r.try_get("path")?,
        side: CommentSide::parse(&side).unwrap_or(CommentSide::New),
        line: n("line")?,
        end_line: n("end_line")?,
        line_text: r.try_get("line_text")?,
        body: r.try_get("body")?,
        author: r.try_get("author")?,
        sent_at: r.try_get("sent_at")?,
        session_id: r.try_get("session_id")?,
        resolved_at: r.try_get("resolved_at")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

impl Store {
    pub async fn insert_review_comment(&self, c: &ReviewComment) -> Result<(), StoreError> {
        let sql =
            format!("INSERT INTO review_comments ({COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&c.id)
            .bind(&c.worktree_id)
            .bind(&c.parent_id)
            .bind(&c.path)
            .bind(c.side.as_str())
            .bind(i64::from(c.line))
            .bind(i64::from(c.end_line))
            .bind(&c.line_text)
            .bind(&c.body)
            .bind(&c.author)
            .bind(c.sent_at)
            .bind(&c.session_id)
            .bind(c.resolved_at)
            .bind(c.created_at)
            .bind(c.updated_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A worktree's comments, oldest first.
    pub async fn list_review_comments(&self, worktree_id: &str) -> Result<Vec<ReviewComment>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM review_comments WHERE worktree_id = ? ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(worktree_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_comment).collect()
    }

    pub async fn get_review_comment(&self, id: &str) -> Result<Option<ReviewComment>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM review_comments WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_comment).transpose()
    }

    /// Rewrites a comment's body and resolved time (callers check what may change).
    pub async fn update_review_comment(&self, c: &ReviewComment) -> Result<(), StoreError> {
        sqlx::query("UPDATE review_comments SET body = ?, resolved_at = ?, updated_at = ? WHERE id = ?")
            .bind(&c.body)
            .bind(c.resolved_at)
            .bind(c.updated_at)
            .bind(&c.id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Deletes a comment (a root's replies go with it).
    pub async fn delete_review_comment(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM review_comments WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Marks these drafts sent to `session_id`. Returns how many were still drafts.
    pub async fn mark_review_comments_sent(
        &self,
        ids: &[String],
        session_id: &str,
        now: i64,
    ) -> Result<u64, StoreError> {
        let mut tx = self.write_tx().await?;
        let mut n = 0;
        for id in ids {
            let r = sqlx::query(
                "UPDATE review_comments SET sent_at = ?, session_id = ?, updated_at = ? WHERE id = ? AND sent_at IS NULL",
            )
            .bind(now)
            .bind(session_id)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            n += r.rows_affected();
        }
        tx.commit().await?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::worktree::Worktree;

    fn comment(id: &str, parent: Option<&str>, at: i64) -> ReviewComment {
        ReviewComment {
            id: id.into(),
            worktree_id: "w".into(),
            parent_id: parent.map(String::from),
            path: "src/a.rs".into(),
            side: CommentSide::Old,
            line: 3,
            end_line: 5,
            line_text: "a\nb\nc".into(),
            body: format!("body {id}"),
            author: "user".into(),
            sent_at: None,
            session_id: None,
            resolved_at: None,
            created_at: at,
            updated_at: at,
        }
    }

    async fn store(dir: &std::path::Path) -> Store {
        let store = store_with_project(dir).await;
        let w = Worktree {
            id: "w".into(),
            project_id: "p".into(),
            branch: "b".into(),
            path: "/r/.worktrees/b".into(),
            base_commit: "abc".into(),
            created_at: 1,
            missing: false,
        };
        store.insert_worktree(&w).await.unwrap();
        store
    }

    #[tokio::test]
    async fn comments_round_trip_are_marked_sent_and_go_with_their_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path()).await;
        store.insert_review_comment(&comment("c2", None, 20)).await.unwrap();
        store.insert_review_comment(&comment("c1", None, 10)).await.unwrap();
        store.insert_review_comment(&comment("r1", Some("c1"), 30)).await.unwrap();
        let all = store.list_review_comments("w").await.unwrap();
        assert_eq!(all.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["c1", "c2", "r1"]);
        assert_eq!(all[0], comment("c1", None, 10));

        let mut c = all[1].clone();
        c.body = "edited".into();
        c.resolved_at = Some(40);
        c.updated_at = 40;
        store.update_review_comment(&c).await.unwrap();
        assert_eq!(store.get_review_comment("c2").await.unwrap(), Some(c));

        let ids = vec!["c1".to_string(), "r1".to_string()];
        assert_eq!(store.mark_review_comments_sent(&ids, "s1", 50).await.unwrap(), 2);
        assert_eq!(store.mark_review_comments_sent(&ids, "s2", 60).await.unwrap(), 0, "already sent");
        let r1 = store.get_review_comment("r1").await.unwrap().unwrap();
        assert_eq!((r1.sent_at, r1.session_id.as_deref()), (Some(50), Some("s1")));

        assert!(store.delete_review_comment("c1").await.unwrap());
        assert_eq!(store.get_review_comment("r1").await.unwrap(), None, "replies go with their root");
        store.delete_worktree("w").await.unwrap();
        assert!(store.list_review_comments("w").await.unwrap().is_empty());
    }
}
