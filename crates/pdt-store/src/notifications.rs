//! Notifications (PDT-108 §3.1): one live row per cause (`dedupe_key`), `seq` bumped on every change for the stream.

use crate::{Store, StoreError};
use pdt_core::notification::{Change, NewNotification, Notification, NotificationKind, Section, Severity, Show};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

const COLUMNS: &str = "id, seq, kind, needs_you, severity, show, project_id, ticket_id, session_id, run_id, \
                       inbox_item_id, title, body, target, dedupe_key, created_at, read_at, cleared_at";

fn corrupt(what: &str, v: &str) -> StoreError {
    StoreError::Corrupt(format!("{what} {v:?}"))
}

fn to_row(r: &SqliteRow) -> Result<Notification, StoreError> {
    let kind: String = r.try_get("kind")?;
    let severity: String = r.try_get("severity")?;
    let show: String = r.try_get("show")?;
    let target: String = r.try_get("target")?;
    Ok(Notification {
        id: r.try_get("id")?,
        seq: r.try_get("seq")?,
        kind: NotificationKind::parse(&kind).ok_or_else(|| corrupt("notification kind", &kind))?,
        needs_you: r.try_get::<i64, _>("needs_you")? != 0,
        severity: Severity::parse(&severity).ok_or_else(|| corrupt("severity", &severity))?,
        show: Show::parse(&show).ok_or_else(|| corrupt("show", &show))?,
        project_id: r.try_get("project_id")?,
        ticket_id: r.try_get("ticket_id")?,
        session_id: r.try_get("session_id")?,
        run_id: r.try_get("run_id")?,
        inbox_item_id: r.try_get("inbox_item_id")?,
        title: r.try_get("title")?,
        body: r.try_get("body")?,
        target: serde_json::from_str(&target).map_err(|e| StoreError::Corrupt(format!("target: {e}")))?,
        dedupe_key: r.try_get("dedupe_key")?,
        created_at: r.try_get("created_at")?,
        read_at: r.try_get("read_at")?,
        cleared_at: r.try_get("cleared_at")?,
    })
}

async fn next_seq(conn: &mut SqliteConnection) -> Result<i64, StoreError> {
    let n: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM notifications").fetch_one(conn).await?;
    Ok(n)
}

async fn get_in(conn: &mut SqliteConnection, id: &str) -> Result<Notification, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM notifications WHERE id = ?");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_one(conn).await?;
    to_row(&row)
}

/// What `GET /api/notifications` lists.
#[derive(Debug, Clone, Default)]
pub struct NotificationFilter {
    pub section: Option<Section>,
    /// Its own and those with no project.
    pub project_id: Option<String>,
    pub unread: bool,
    /// Only live rows (Needs you lists only these).
    pub live: bool,
    /// Only rows made before this (ms): the next page.
    pub before: Option<i64>,
    pub limit: i64,
}

impl Store {
    /// Makes a notification, or updates the live row of its `dedupe_key` (title, body, created_at; unread again).
    pub async fn put_notification(
        &self,
        n: &NewNotification,
        show: Show,
        now: i64,
    ) -> Result<(Notification, Change), StoreError> {
        let mut tx = self.write_tx().await?;
        let seq = next_seq(&mut tx).await?;
        let live: Option<String> =
            sqlx::query_scalar("SELECT id FROM notifications WHERE dedupe_key = ? AND cleared_at IS NULL")
                .bind(&n.dedupe_key)
                .fetch_optional(&mut *tx)
                .await?;
        let target = serde_json::to_string(&n.target).unwrap_or_else(|_| "{\"kind\":\"none\"}".into());
        let (id, change) = match live {
            Some(id) => {
                sqlx::query(
                    "UPDATE notifications SET seq = ?, severity = ?, show = ?, title = ?, body = ?, target = ?, \
                     created_at = ?, read_at = NULL WHERE id = ?",
                )
                .bind(seq)
                .bind(n.severity.as_str())
                .bind(show.as_str())
                .bind(self.redactor.string(&n.title))
                .bind(self.redactor.string(&n.body))
                .bind(&target)
                .bind(now)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
                (id, Change::Updated)
            }
            None => {
                let id = uuid::Uuid::now_v7().to_string();
                sqlx::query(
                    "INSERT INTO notifications (id, seq, kind, needs_you, severity, show, project_id, ticket_id, \
                     session_id, run_id, inbox_item_id, title, body, target, dedupe_key, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(seq)
                .bind(n.kind.as_str())
                .bind(n.kind.needs_you())
                .bind(n.severity.as_str())
                .bind(show.as_str())
                .bind(&n.project_id)
                .bind(&n.ticket_id)
                .bind(&n.session_id)
                .bind(&n.run_id)
                .bind(&n.inbox_item_id)
                .bind(self.redactor.string(&n.title))
                .bind(self.redactor.string(&n.body))
                .bind(&target)
                .bind(&n.dedupe_key)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                (id, Change::Created)
            }
        };
        let row = get_in(&mut tx, &id).await?;
        tx.commit().await?;
        Ok((row, change))
    }

    /// Clears the live row of a cause; None when there was none.
    pub async fn clear_notification(&self, dedupe_key: &str, now: i64) -> Result<Option<Notification>, StoreError> {
        let mut tx = self.write_tx().await?;
        let seq = next_seq(&mut tx).await?;
        let id: Option<String> = sqlx::query_scalar(
            "UPDATE notifications SET seq = ?, cleared_at = ? WHERE dedupe_key = ? AND cleared_at IS NULL RETURNING id",
        )
        .bind(seq)
        .bind(now)
        .bind(dedupe_key)
        .fetch_optional(&mut *tx)
        .await?;
        let row = match id {
            Some(id) => Some(get_in(&mut tx, &id).await?),
            None => None,
        };
        tx.commit().await?;
        Ok(row)
    }

    pub async fn get_notification(&self, id: &str) -> Result<Option<Notification>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM notifications WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_row).transpose()
    }

    /// Every live row (not cleared), oldest first.
    pub async fn live_notifications(&self) -> Result<Vec<Notification>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM notifications WHERE cleared_at IS NULL ORDER BY created_at, seq");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_row).collect()
    }

    /// Rows newest first.
    pub async fn list_notifications(&self, f: &NotificationFilter) -> Result<Vec<Notification>, StoreError> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!("SELECT {COLUMNS} FROM notifications WHERE 1 = 1"));
        if let Some(s) = f.section {
            qb.push(" AND needs_you = ").push_bind(s == Section::NeedsYou);
        }
        if let Some(p) = &f.project_id {
            qb.push(" AND (project_id = ").push_bind(p.clone()).push(" OR project_id IS NULL)");
        }
        if f.unread {
            qb.push(" AND read_at IS NULL");
        }
        if f.live {
            qb.push(" AND cleared_at IS NULL");
        }
        if let Some(b) = f.before {
            qb.push(" AND created_at < ").push_bind(b);
        }
        qb.push(" ORDER BY created_at DESC, seq DESC LIMIT ").push_bind(f.limit.clamp(1, 500));
        let rows = qb.build().fetch_all(&self.pool).await?;
        rows.iter().map(to_row).collect()
    }

    /// Updates neither read nor cleared (the Inbox's dot).
    pub async fn unread_updates(&self, project_id: Option<&str>) -> Result<u32, StoreError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications WHERE needs_you = 0 AND read_at IS NULL AND cleared_at IS NULL \
             AND (? IS NULL OR project_id = ? OR project_id IS NULL)",
        )
        .bind(project_id)
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Rows changed after `seq`, in order (the stream's catch-up).
    pub async fn notifications_after(&self, seq: i64, limit: i64) -> Result<Vec<Notification>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM notifications WHERE seq > ? ORDER BY seq LIMIT ?");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(seq).bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(to_row).collect()
    }

    pub async fn last_notification_seq(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM notifications").fetch_one(&self.pool).await?)
    }

    /// Marks one row read. False when there is no such row.
    pub async fn read_notification(&self, id: &str, now: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE notifications SET read_at = COALESCE(read_at, ?) WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Marks rows read: a section's, a session's or an Inbox item's (all of them with no filter).
    pub async fn read_notifications(
        &self,
        section: Option<Section>,
        session_id: Option<&str>,
        inbox_item_id: Option<&str>,
        now: i64,
    ) -> Result<u64, StoreError> {
        let mut qb = QueryBuilder::<Sqlite>::new("UPDATE notifications SET read_at = ");
        qb.push_bind(now).push(" WHERE read_at IS NULL");
        if let Some(s) = section {
            qb.push(" AND needs_you = ").push_bind(s == Section::NeedsYou);
        }
        if let Some(s) = session_id {
            qb.push(" AND session_id = ").push_bind(s.to_string());
        }
        if let Some(i) = inbox_item_id {
            qb.push(" AND inbox_item_id = ").push_bind(i.to_string());
        }
        Ok(qb.build().execute(&self.pool).await?.rows_affected())
    }

    /// Deletes rows made before `before`, except live Needs-you rows (their cause still waits). Returns how many went.
    pub async fn prune_notifications(&self, before: i64) -> Result<u64, StoreError> {
        let r = sqlx::query(
            "DELETE FROM notifications WHERE created_at < ? AND NOT (needs_you = 1 AND cleared_at IS NULL)",
        )
        .bind(before)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::notification::NotificationTarget;

    fn new(kind: NotificationKind, key: &str) -> NewNotification {
        NewNotification {
            project_id: Some("p".into()),
            session_id: Some("s".into()),
            ..NewNotification::new(kind, key, "Title", NotificationTarget::None)
        }
    }

    #[tokio::test]
    async fn one_live_row_per_cause_and_every_change_moves_seq() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let (a, c) = store.put_notification(&new(NotificationKind::PrCiFailed, "k"), Show::Inbox, 10).await.unwrap();
        assert_eq!((c, a.seq, a.needs_you, a.show), (Change::Created, 1, false, Show::Inbox));
        store.read_notification(&a.id, 11).await.unwrap();
        let mut again = new(NotificationKind::PrCiFailed, "k");
        again.title = "Again".into();
        let (b, c) = store.put_notification(&again, Show::Inbox, 20).await.unwrap();
        assert_eq!((c, b.id.as_str(), b.title.as_str(), b.created_at), (Change::Updated, a.id.as_str(), "Again", 20));
        assert_eq!((b.seq, b.read_at), (2, None), "unread again, with a new seq");

        let cleared = store.clear_notification("k", 30).await.unwrap().unwrap();
        assert_eq!((cleared.seq, cleared.cleared_at), (3, Some(30)));
        assert!(store.clear_notification("k", 31).await.unwrap().is_none(), "nothing live");
        let (c2, c) = store.put_notification(&new(NotificationKind::PrCiFailed, "k"), Show::Inbox, 40).await.unwrap();
        assert_eq!(c, Change::Created, "a cleared row leaves room for a new one");
        assert_ne!(c2.id, a.id);

        let after = store.notifications_after(1, 10).await.unwrap();
        assert_eq!(after.iter().map(|n| n.seq).collect::<Vec<_>>(), [3, 4]);
        assert_eq!(Change::of(&after[0]), Change::Cleared);
        assert_eq!(store.last_notification_seq().await.unwrap(), 4);
    }

    #[tokio::test]
    async fn sections_unread_and_pruning() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let (q, _) =
            store.put_notification(&new(NotificationKind::SessionNeedsYou, "q"), Show::Desktop, 5).await.unwrap();
        store.put_notification(&new(NotificationKind::PrMerged, "m"), Show::Inbox, 6).await.unwrap();
        store.put_notification(&new(NotificationKind::WorkflowFailed, "w"), Show::Inbox, 7).await.unwrap();
        let f = |section| NotificationFilter { section: Some(section), limit: 50, ..Default::default() };
        assert_eq!(store.list_notifications(&f(Section::NeedsYou)).await.unwrap().len(), 1);
        let updates = store.list_notifications(&f(Section::Updates)).await.unwrap();
        assert_eq!(updates.iter().map(|n| n.dedupe_key.as_str()).collect::<Vec<_>>(), ["w", "m"], "newest first");
        assert_eq!(store.unread_updates(None).await.unwrap(), 2);
        assert_eq!(store.unread_updates(Some("other")).await.unwrap(), 0);
        assert_eq!(store.read_notifications(Some(Section::Updates), None, None, 8).await.unwrap(), 2);
        assert_eq!(store.unread_updates(None).await.unwrap(), 0);
        assert!(store.get_notification(&q.id).await.unwrap().unwrap().read_at.is_none(), "only Updates");

        assert_eq!(store.prune_notifications(100).await.unwrap(), 2, "the live Needs-you row stays");
        store.clear_notification("q", 9).await.unwrap();
        assert_eq!(store.prune_notifications(100).await.unwrap(), 1);
    }
}
