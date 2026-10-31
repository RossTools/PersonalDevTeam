//! Watches (PDT-319): who hears about a ticket or a workflow run, and how far each has looked.

use crate::{Store, StoreError};
use pdt_core::watch::{Watch, WatchEvent, WatchNote, WatchTarget};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const COLUMNS: &str =
    "id, target, target_id, session_id, events, once, cursor, inbox_since, run_status, pending, created_at, ended_at";

fn to_watch(r: &SqliteRow) -> Result<Watch, StoreError> {
    let target: String = r.try_get("target")?;
    let events: String = r.try_get("events")?;
    let pending: String = r.try_get("pending")?;
    Ok(Watch {
        id: r.try_get("id")?,
        target: WatchTarget::parse(&target).ok_or_else(|| StoreError::Corrupt(format!("watch target {target:?}")))?,
        target_id: r.try_get("target_id")?,
        session_id: r.try_get("session_id")?,
        events: serde_json::from_str::<Vec<WatchEvent>>(&events)
            .map_err(|e| StoreError::Corrupt(format!("watch events: {e}")))?,
        once: r.try_get::<i64, _>("once")? != 0,
        cursor: r.try_get("cursor")?,
        inbox_since: r.try_get("inbox_since")?,
        run_status: r.try_get("run_status")?,
        pending: serde_json::from_str::<Vec<WatchNote>>(&pending).unwrap_or_default(),
        created_at: r.try_get("created_at")?,
        ended_at: r.try_get("ended_at")?,
    })
}

fn to_json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

impl Store {
    /// Adds a watch, or, when the watcher already watches the target, sets that one's events and `once`: the watch.
    pub async fn put_watch(&self, w: &Watch) -> Result<Watch, StoreError> {
        let mut tx = self.write_tx().await?;
        let sql = format!(
            "SELECT {COLUMNS} FROM watches WHERE target = ? AND target_id = ? AND COALESCE(session_id, '') = ? \
             AND ended_at IS NULL"
        );
        let found = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(w.target.as_str())
            .bind(&w.target_id)
            .bind(w.session_id.as_deref().unwrap_or(""))
            .fetch_optional(&mut *tx)
            .await?;
        let id = match found {
            Some(r) => {
                let id: String = r.try_get("id")?;
                sqlx::query("UPDATE watches SET events = ?, once = ? WHERE id = ?")
                    .bind(to_json(&w.events))
                    .bind(i64::from(w.once))
                    .bind(&id)
                    .execute(&mut *tx)
                    .await?;
                id
            }
            None => {
                sqlx::query(
                    "INSERT INTO watches (id, target, target_id, session_id, events, once, cursor, inbox_since, \
                     run_status, pending, created_at, ended_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
                )
                .bind(&w.id)
                .bind(w.target.as_str())
                .bind(&w.target_id)
                .bind(&w.session_id)
                .bind(to_json(&w.events))
                .bind(i64::from(w.once))
                .bind(w.cursor)
                .bind(w.inbox_since)
                .bind(&w.run_status)
                .bind(to_json(&w.pending))
                .bind(w.created_at)
                .execute(&mut *tx)
                .await?;
                w.id.clone()
            }
        };
        tx.commit().await?;
        self.get_watch(&id).await?.ok_or_else(|| StoreError::Corrupt("a watch went missing".into()))
    }

    pub async fn get_watch(&self, id: &str) -> Result<Option<Watch>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM watches WHERE id = ?");
        let r = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        r.as_ref().map(to_watch).transpose()
    }

    /// Every watch that hasn't ended, oldest first.
    pub async fn active_watches(&self) -> Result<Vec<Watch>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM watches WHERE ended_at IS NULL ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_watch).collect()
    }

    /// The watches of a target that haven't ended, oldest first.
    pub async fn watches_of(&self, target: WatchTarget, target_id: &str) -> Result<Vec<Watch>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM watches WHERE target = ? AND target_id = ? AND ended_at IS NULL \
             ORDER BY created_at, id"
        );
        let rows =
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(target.as_str()).bind(target_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_watch).collect()
    }

    /// A watcher's watches that haven't ended (None: the user's), oldest first.
    pub async fn watches_by(&self, session_id: Option<&str>) -> Result<Vec<Watch>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM watches WHERE COALESCE(session_id, '') = ? AND ended_at IS NULL \
             ORDER BY created_at, id"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id.unwrap_or("")).fetch_all(&self.pool).await?;
        rows.iter().map(to_watch).collect()
    }

    /// Records how far a watch has looked and what it has to tell, in one write (so a restart neither repeats nor
    /// misses an event). False when it ended meanwhile.
    pub async fn set_watch_progress(&self, w: &Watch) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE watches SET cursor = ?, inbox_since = ?, run_status = ?, pending = ? \
             WHERE id = ? AND ended_at IS NULL",
        )
        .bind(w.cursor)
        .bind(w.inbox_since)
        .bind(&w.run_status)
        .bind(to_json(&w.pending))
        .bind(&w.id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Ends a watch; false when it had ended.
    pub async fn end_watch(&self, id: &str, now: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE watches SET ended_at = ?, pending = '[]' WHERE id = ? AND ended_at IS NULL")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Moves a conversation's watches to the one that took its place (a PM Reset): those of targets the new one
    /// already watches end instead. How many moved.
    pub async fn move_watches(&self, from: &str, to: &str, now: i64) -> Result<u64, StoreError> {
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "UPDATE watches SET ended_at = ? WHERE session_id = ? AND ended_at IS NULL AND EXISTS \
             (SELECT 1 FROM watches w WHERE w.session_id = ? AND w.ended_at IS NULL AND w.target = watches.target \
              AND w.target_id = watches.target_id)",
        )
        .bind(now)
        .bind(from)
        .bind(to)
        .execute(&mut *tx)
        .await?;
        let r = sqlx::query("UPDATE watches SET session_id = ? WHERE session_id = ? AND ended_at IS NULL")
            .bind(to)
            .bind(from)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(r.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;

    fn watch(id: &str, target: &str, session: Option<&str>) -> Watch {
        Watch {
            id: id.into(),
            target: WatchTarget::Ticket,
            target_id: target.into(),
            session_id: session.map(String::from),
            events: vec![WatchEvent::Finished],
            once: true,
            cursor: 3,
            inbox_since: 10,
            run_status: None,
            pending: vec![],
            created_at: 1,
            ended_at: None,
        }
    }

    #[tokio::test]
    async fn one_watch_per_watcher_and_target() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let a = store.put_watch(&watch("a", "t", Some("pm"))).await.unwrap();
        assert_eq!((a.cursor, a.once), (3, true));
        let mut again = watch("b", "t", Some("pm"));
        again.events = vec![WatchEvent::Comment];
        again.once = false;
        let b = store.put_watch(&again).await.unwrap();
        assert_eq!(b.id, "a", "the same watch, with the new settings");
        assert_eq!((b.events.clone(), b.once), (vec![WatchEvent::Comment], false));
        store.put_watch(&watch("u", "t", None)).await.unwrap();
        assert_eq!(store.watches_of(WatchTarget::Ticket, "t").await.unwrap().len(), 2, "the user's apart");
        assert_eq!(store.watches_by(None).await.unwrap()[0].id, "u");

        let mut p = b.clone();
        p.cursor = 9;
        p.pending = vec![WatchNote { event: WatchEvent::Comment, at: 5, line: "x".into(), ends: false }];
        assert!(store.set_watch_progress(&p).await.unwrap());
        assert_eq!(store.get_watch("a").await.unwrap().unwrap().pending, p.pending);
        assert!(store.end_watch("a", 7).await.unwrap());
        assert!(!store.end_watch("a", 8).await.unwrap());
        assert!(!store.set_watch_progress(&p).await.unwrap(), "an ended watch stays ended");
        assert_eq!(store.get_watch("a").await.unwrap().unwrap().ended_at, Some(7));
        assert_eq!(store.put_watch(&watch("c", "t", Some("pm"))).await.unwrap().id, "c", "a new one after the end");
    }

    #[tokio::test]
    async fn watches_move_to_the_conversation_that_took_over() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.put_watch(&watch("a", "t1", Some("old"))).await.unwrap();
        store.put_watch(&watch("b", "t2", Some("old"))).await.unwrap();
        store.put_watch(&watch("c", "t2", Some("new"))).await.unwrap();
        assert_eq!(store.move_watches("old", "new", 5).await.unwrap(), 1);
        let ids: Vec<String> = store.watches_by(Some("new")).await.unwrap().into_iter().map(|w| w.id).collect();
        assert_eq!(ids, ["a", "c"]);
        assert!(store.watches_by(Some("old")).await.unwrap().is_empty());
        assert_eq!(store.get_watch("b").await.unwrap().unwrap().ended_at, Some(5));
    }
}
