//! A ticket's attachments' metadata (PDT-300); pdt-api keeps the files.

use crate::{Store, StoreError};
use pdt_core::ticket::Actor;
use pdt_core::ticket_attachment::{MAX_PER_TICKET, TicketAttachment};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const SELECT: &str =
    "SELECT id, ticket_id, name, media_type, size, actor, agent, event_id, created_at FROM ticket_attachments";

fn to_attachment(r: &SqliteRow) -> Result<TicketAttachment, StoreError> {
    let actor: String = r.try_get("actor")?;
    Ok(TicketAttachment {
        id: r.try_get("id")?,
        ticket_id: r.try_get("ticket_id")?,
        name: r.try_get("name")?,
        media_type: r.try_get("media_type")?,
        size: r.try_get("size")?,
        actor: Actor::parse(&actor).unwrap_or(Actor::Agent),
        agent: r.try_get::<Option<String>, _>("agent")?.and_then(|j| serde_json::from_str(&j).ok()),
        event_id: r.try_get("event_id")?,
        created_at: r.try_get("created_at")?,
    })
}

/// The ticket's attachments `text` links to, in the order it first shows them (a comment's, PDT-341).
pub(crate) async fn referenced(
    conn: &mut sqlx::SqliteConnection,
    ticket_id: &str,
    text: &str,
) -> Result<Vec<TicketAttachment>, StoreError> {
    let mut out = Vec::new();
    for id in pdt_core::ticket_attachment::referenced_ids(text, ticket_id) {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE ticket_id = ? AND id = ?")))
            .bind(ticket_id)
            .bind(&id)
            .fetch_optional(&mut *conn)
            .await?;
        if let Some(r) = row {
            out.push(to_attachment(&r)?);
        }
    }
    Ok(out)
}

/// What adding an attachment did.
#[derive(Debug, PartialEq)]
pub enum AddAttachment {
    Added,
    NoTicket,
    /// The ticket holds [`MAX_PER_TICKET`] already.
    Full,
}

impl Store {
    /// Records an attachment whose file is in place, unless its ticket is gone or full.
    pub async fn add_ticket_attachment(&self, a: &TicketAttachment) -> Result<AddAttachment, StoreError> {
        let mut tx = self.write_tx().await?;
        let count: Option<(i64,)> = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM ticket_attachments WHERE ticket_id = t.id) FROM tickets t WHERE t.id = ?",
        )
        .bind(&a.ticket_id)
        .fetch_optional(&mut *tx)
        .await?;
        match count {
            None => return Ok(AddAttachment::NoTicket),
            Some((n,)) if n >= MAX_PER_TICKET as i64 => return Ok(AddAttachment::Full),
            Some(_) => {}
        }
        sqlx::query(
            "INSERT INTO ticket_attachments (id, ticket_id, name, media_type, size, actor, agent, event_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&a.id)
        .bind(&a.ticket_id)
        .bind(&a.name)
        .bind(&a.media_type)
        .bind(a.size)
        .bind(a.actor.as_str())
        .bind(a.agent.as_ref().map(|g| serde_json::to_string(g).expect("an agent serializes")))
        .bind(&a.event_id)
        .bind(a.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(AddAttachment::Added)
    }

    /// A ticket's attachments, oldest first.
    pub async fn ticket_attachments(&self, ticket_id: &str) -> Result<Vec<TicketAttachment>, StoreError> {
        sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE ticket_id = ? ORDER BY created_at, id")))
            .bind(ticket_id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(to_attachment)
            .collect()
    }

    pub async fn get_ticket_attachment(
        &self,
        ticket_id: &str,
        id: &str,
    ) -> Result<Option<TicketAttachment>, StoreError> {
        sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE ticket_id = ? AND id = ?")))
            .bind(ticket_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(to_attachment)
            .transpose()
    }

    /// Removes an attachment's row; false if there was none.
    pub async fn delete_ticket_attachment(&self, ticket_id: &str, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM ticket_attachments WHERE ticket_id = ? AND id = ?")
            .bind(ticket_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Names the comment the attachments came with.
    pub async fn set_attachments_event(&self, ids: &[String], event_id: &str) -> Result<(), StoreError> {
        for id in ids {
            sqlx::query("UPDATE ticket_attachments SET event_id = ? WHERE id = ?")
                .bind(event_id)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Which of these ticket ids still exist (pdt-api removes the others' attachment folders).
    pub async fn existing_ticket_ids(&self, ids: &[String]) -> Result<Vec<String>, StoreError> {
        let mut out = Vec::new();
        for id in ids {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT id FROM tickets WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
            out.extend(row.map(|(id,)| id));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::ticket::NewTicket;

    async fn store_with_ticket(dir: &std::path::Path) -> (Store, String) {
        let store = crate::worktrees::tests::store_with_project(dir).await;
        let t = NewTicket { title: "T".into(), ..Default::default() }.validate().unwrap();
        store.create_ticket("t1", "p", &t, 1).await.unwrap();
        (store, "t1".into())
    }

    fn attachment(id: &str, ticket: &str) -> TicketAttachment {
        TicketAttachment {
            id: id.into(),
            ticket_id: ticket.into(),
            name: "shot.png".into(),
            media_type: "image/png".into(),
            size: 10,
            actor: Actor::User,
            agent: None,
            event_id: None,
            created_at: 5,
        }
    }

    #[tokio::test]
    async fn attachments_are_kept_listed_and_removed_with_their_ticket() {
        let dir = tempfile::tempdir().unwrap();
        let (store, t) = store_with_ticket(dir.path()).await;
        assert_eq!(store.add_ticket_attachment(&attachment("a1", &t)).await.unwrap(), AddAttachment::Added);
        assert_eq!(store.add_ticket_attachment(&attachment("a2", "nope")).await.unwrap(), AddAttachment::NoTicket);
        store.set_attachments_event(&["a1".into()], "e1").await.unwrap();
        let list = store.ticket_attachments(&t).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].event_id.as_deref(), Some("e1"));
        assert!(store.get_ticket_attachment(&t, "a1").await.unwrap().is_some());
        assert!(store.get_ticket_attachment("other", "a1").await.unwrap().is_none());
        store.delete_ticket(&t).await.unwrap();
        assert!(store.ticket_attachments(&t).await.unwrap().is_empty());
        assert!(store.existing_ticket_ids(&[t]).await.unwrap().is_empty());
    }

    /// PDT-341: a comment's event says who wrote it (the PM's persona) and which attachments it links to.
    #[tokio::test]
    async fn a_comments_event_names_its_author_and_attachments() {
        use pdt_core::ticket::{AgentBy, By};
        let dir = tempfile::tempdir().unwrap();
        let (store, t) = store_with_ticket(dir.path()).await;
        store.add_ticket_attachment(&attachment("a1", &t)).await.unwrap();
        let pm = By::agent(AgentBy {
            session_id: "s1".into(),
            project_id: None,
            persona: Some("PM".into()),
            from_ticket: None,
            from_ticket_id: None,
        });
        let body =
            "Look at this.\n\n![shot.png](/api/tickets/t1/attachments/a1) [gone](/api/tickets/t1/attachments/a9)";
        store.add_comment_by(&t, body, pm, None, 6).await.unwrap();
        store.add_comment_by(&t, "And this.", Actor::User, None, 7).await.unwrap();
        store.add_comment_by(&t, "Done.", Actor::Agent, None, 8).await.unwrap();
        let log = store
            .list_log_events(&crate::EventFilter {
                kind: Some("ticket.commented".into()),
                limit: 5,
                ..Default::default()
            })
            .await
            .unwrap();
        let by: Vec<(&str, &str)> =
            log.iter().map(|e| (e.payload["author"].as_str().unwrap(), e.payload["actor"].as_str().unwrap())).collect();
        assert_eq!(by, [("PM", "agent"), ("user", "user"), ("agent", "agent")]);
        assert_eq!(
            log[0].payload["attachments"],
            serde_json::json!([{"id": "a1", "name": "shot.png", "mediaType": "image/png", "fileName": "a1.png"}]),
            "only the ticket's own attachments that exist"
        );
        assert_eq!(log[1].payload["attachments"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn a_ticket_holds_a_limited_number() {
        let dir = tempfile::tempdir().unwrap();
        let (store, t) = store_with_ticket(dir.path()).await;
        for i in 0..MAX_PER_TICKET {
            store.add_ticket_attachment(&attachment(&format!("a{i}"), &t)).await.unwrap();
        }
        assert_eq!(store.add_ticket_attachment(&attachment("over", &t)).await.unwrap(), AddAttachment::Full);
        assert!(store.delete_ticket_attachment(&t, "a0").await.unwrap());
        assert!(!store.delete_ticket_attachment(&t, "a0").await.unwrap());
        assert_eq!(store.add_ticket_attachment(&attachment("over", &t)).await.unwrap(), AddAttachment::Added);
    }
}
