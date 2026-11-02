//! Links between tickets and to issues in other trackers.

use super::*;

/// A ticket's link to an issue in another tracker, as stored (with the connection it is read through).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExternalLink {
    pub ticket_id: String,
    pub system: String,
    pub connection_id: String,
    pub key: String,
    pub url: String,
}

/// `linked`/`unlinked` on both tickets, each naming the other.
pub(super) async fn link_events(
    conn: &mut SqliteConnection,
    event: EventKind,
    kind: LinkKind,
    (from, from_key): (&str, &str),
    (to, to_key): (&str, &str),
    by: &By,
    now: i64,
) -> Result<(), StoreError> {
    let data = |dir: LinkDirection, other: &str, key: &str| {
        Some(json!({"kind": kind, "direction": dir, "ticketId": other, "key": key}))
    };
    insert_event(conn, from, event, by, None, data(LinkDirection::Outgoing, to, to_key), now).await?;
    insert_event(conn, to, event, by, None, data(LinkDirection::Incoming, from, from_key), now).await?;
    Ok(())
}

impl Store {
    /// Adds a ticket linked to an external issue (`link.ticket_id` is ignored), in one transaction, on behalf of `by`.
    /// It is `external` whatever `t.source` says (PDT-201). `Ok(Err(id))` when a ticket already links that issue
    /// (nothing is made): linking is the dedupe, so an issue is never imported twice.
    pub async fn create_linked_ticket(
        &self,
        id: &str,
        project_id: &str,
        t: &ValidNewTicket,
        link: &StoredExternalLink,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Result<Ticket, String>> {
        let t = &ValidNewTicket { source: TicketSource::External, ..t.clone() };
        let mut tx = self.write_tx().await?;
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT ticket_id FROM ticket_links_external WHERE system = ? AND connection_id = ? AND key = ?",
        )
        .bind(&link.system)
        .bind(&link.connection_id)
        .bind(&link.key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((other,)) = existing {
            return Ok(Err(other));
        }
        insert_ticket(&mut tx, id, project_id, t, &by.into(), Made::default(), now).await?;
        sqlx::query(
            "INSERT INTO ticket_links_external (ticket_id, system, connection_id, key, url, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&link.system)
        .bind(&link.connection_id)
        .bind(&link.key)
        .bind(&link.url)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Ok(self.get_ticket(id).await?.ok_or(TicketError::NotFound("ticket"))?))
    }

    /// Links ticket `link.ticket_id` to an external issue it was filed as (PDT-221), on behalf of `by`. Its source is
    /// unchanged: the issue came from the ticket, not from outside. `Ok(Err(id))` when a ticket already links that
    /// issue (nothing changes); NotFound("ticket") when the ticket is gone.
    pub async fn link_external(
        &self,
        link: &StoredExternalLink,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Result<Ticket, String>> {
        let mut tx = self.write_tx().await?;
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM tickets WHERE id = ?")
            .bind(&link.ticket_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(TicketError::NotFound("ticket"));
        }
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT ticket_id FROM ticket_links_external WHERE system = ? AND connection_id = ? AND key = ?",
        )
        .bind(&link.system)
        .bind(&link.connection_id)
        .bind(&link.key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((other,)) = existing {
            return Ok(Err(other));
        }
        sqlx::query(
            "INSERT INTO ticket_links_external (ticket_id, system, connection_id, key, url, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&link.ticket_id)
        .bind(&link.system)
        .bind(&link.connection_id)
        .bind(&link.key)
        .bind(&link.url)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        touch(&mut tx, &link.ticket_id, now).await?;
        let data = json!({"system": link.system, "key": link.key, "url": link.url});
        insert_event(&mut tx, &link.ticket_id, EventKind::Linked, by, None, Some(data), now).await?;
        tx.commit().await?;
        Ok(Ok(self.get_ticket(&link.ticket_id).await?.ok_or(TicketError::NotFound("ticket"))?))
    }

    /// The ticket linked to an external issue.
    pub async fn ticket_for_external(
        &self,
        system: &str,
        connection_id: &str,
        key: &str,
    ) -> Result<Option<String>, StoreError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT ticket_id FROM ticket_links_external WHERE system = ? AND connection_id = ? AND key = ?",
        )
        .bind(system)
        .bind(connection_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(t,)| t))
    }

    /// A ticket's links to issues of `system`.
    pub async fn external_links(&self, ticket_id: &str, system: &str) -> Result<Vec<StoredExternalLink>, StoreError> {
        let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT ticket_id, system, connection_id, key, url FROM ticket_links_external \
             WHERE ticket_id = ? AND system = ? ORDER BY key",
        )
        .bind(ticket_id)
        .bind(system)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(ticket_id, system, connection_id, key, url)| StoredExternalLink {
                ticket_id,
                system,
                connection_id,
                key,
                url,
            })
            .collect())
    }

    /// Links `from` to `to` (both in one project). A `blocks` link that would close a cycle is refused with the
    /// cycle's keys. Linking twice (or `relates` in either direction) is a no-op. None if `from` doesn't exist.
    pub async fn add_link(&self, from: &str, to: &str, kind: LinkKind, now: i64) -> TResult<Option<Ticket>> {
        self.add_link_by(from, to, kind, Actor::User, now).await
    }

    /// [`Store::add_link`] on behalf of `actor` (the PM's `link_tickets`), who the `linked` events name.
    pub async fn add_link_by(
        &self,
        from: &str,
        to: &str,
        kind: LinkKind,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Option<Ticket>> {
        if from == to {
            return Err(TicketError::Invalid("a ticket can't link to itself".into()));
        }
        let mut tx = self.write_tx().await?;
        let Some((project, from_key, _)) = ticket_head(&mut tx, from).await? else { return Ok(None) };
        let Some((to_project, to_key, _)) = ticket_head(&mut tx, to).await? else {
            return Err(TicketError::Invalid("the ticket to link to does not exist".into()));
        };
        if project != to_project {
            return Err(TicketError::Invalid("links join tickets of the same project".into()));
        }
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT kind FROM ticket_links WHERE kind = ?1 AND \
             ((from_id = ?2 AND to_id = ?3) OR (?1 = 'relates' AND from_id = ?3 AND to_id = ?2))",
        )
        .bind(kind.as_str())
        .bind(from)
        .bind(to)
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_some() {
            drop(tx);
            return Ok(self.get_ticket(from).await?);
        }
        if kind == LinkKind::Blocks {
            let edges: Vec<(String, String)> = sqlx::query_as(
                "SELECT l.from_id, l.to_id FROM ticket_links l JOIN tickets t ON t.id = l.from_id \
                 WHERE t.project_id = ? AND l.kind = 'blocks'",
            )
            .bind(&project)
            .fetch_all(&mut *tx)
            .await?;
            if let Some(cycle) = would_cycle(&edges, from, to) {
                let mut keys = Vec::with_capacity(cycle.len());
                for id in cycle {
                    keys.push(ticket_head(&mut tx, &id).await?.map(|h| h.1).unwrap_or(id));
                }
                return Err(TicketError::Cycle(keys));
            }
        }
        sqlx::query("INSERT INTO ticket_links (from_id, to_id, kind, created_at) VALUES (?, ?, ?, ?)")
            .bind(from)
            .bind(to)
            .bind(kind.as_str())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        // A link is a change on both sides: auto-start's quiet period starts again (PDT-146).
        touch(&mut tx, from, now).await?;
        touch(&mut tx, to, now).await?;
        link_events(&mut tx, EventKind::Linked, kind, (from, &from_key), (to, &to_key), &by.into(), now).await?;
        tx.commit().await?;
        Ok(self.get_ticket(from).await?)
    }

    /// Removes a link (`relates` in either direction). NotFound("link") if there is none; None if `from` doesn't
    /// exist.
    pub async fn remove_link(&self, from: &str, to: &str, kind: LinkKind, now: i64) -> TResult<Option<Ticket>> {
        self.remove_link_by(from, to, kind, Actor::User, now).await
    }

    /// [`Store::remove_link`] on behalf of `actor` (the PM's `unlink_tickets`), who the `unlinked` events name.
    pub async fn remove_link_by(
        &self,
        from: &str,
        to: &str,
        kind: LinkKind,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Option<Ticket>> {
        let mut tx = self.write_tx().await?;
        let Some((_, from_key, from_stage)) = ticket_head(&mut tx, from).await? else { return Ok(None) };
        let r = sqlx::query(
            "DELETE FROM ticket_links WHERE kind = ?1 AND \
             ((from_id = ?2 AND to_id = ?3) OR (?1 = 'relates' AND from_id = ?3 AND to_id = ?2))",
        )
        .bind(kind.as_str())
        .bind(from)
        .bind(to)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 0 {
            return Err(TicketError::NotFound("link"));
        }
        let to_key = ticket_head(&mut tx, to).await?.map(|h| h.1).unwrap_or_default();
        touch(&mut tx, from, now).await?;
        touch(&mut tx, to, now).await?;
        link_events(&mut tx, EventKind::Unlinked, kind, (from, &from_key), (to, &to_key), &by.into(), now).await?;
        if kind == LinkKind::Blocks && !from_stage.is_closed() {
            let sql = format!("SELECT {BLOCKED_BY} FROM tickets t WHERE t.id = ?");
            let (n,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(to).fetch_one(&mut *tx).await?;
            if n == 0 {
                let data = json!({"by": from, "reason": "unlinked"});
                insert_event(&mut tx, to, EventKind::Unblocked, Actor::System, None, Some(data), now).await?;
            }
        }
        tx.commit().await?;
        Ok(self.get_ticket(from).await?)
    }
}
