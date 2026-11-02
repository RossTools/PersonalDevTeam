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
    pub origin: LinkOrigin,
    /// Done closes (or moves) the issue.
    pub close_on_done: bool,
    /// The issue's title and state, when they are known.
    pub title: Option<String>,
    pub state: Option<IssueState>,
}

impl StoredExternalLink {
    /// A link made `origin`'s way, closing the issue on Done as such links do unless told otherwise.
    pub fn new(
        ticket_id: impl Into<String>,
        system: impl Into<String>,
        connection_id: impl Into<String>,
        key: impl Into<String>,
        url: impl Into<String>,
        origin: LinkOrigin,
    ) -> Self {
        Self {
            ticket_id: ticket_id.into(),
            system: system.into(),
            connection_id: connection_id.into(),
            key: key.into(),
            url: url.into(),
            origin,
            close_on_done: origin.closes_by_default(),
            title: None,
            state: None,
        }
    }
}

const STORED_LINK: &str = "SELECT ticket_id, system, connection_id, key, url, origin, close_on_done, title, state \
                           FROM ticket_links_external";

fn to_stored_link(r: &sqlx::sqlite::SqliteRow) -> Result<StoredExternalLink, StoreError> {
    let origin: String = r.try_get(5)?;
    let state: Option<String> = r.try_get(8)?;
    Ok(StoredExternalLink {
        ticket_id: r.try_get(0)?,
        system: r.try_get(1)?,
        connection_id: r.try_get(2)?,
        key: r.try_get(3)?,
        url: r.try_get(4)?,
        origin: LinkOrigin::parse(&origin).ok_or_else(|| corrupt("link origin", &origin))?,
        close_on_done: r.try_get(6)?,
        title: r.try_get(7)?,
        state: state.as_deref().and_then(IssueState::parse),
    })
}

/// Inserts a link to an issue as `link` says.
async fn insert_link(conn: &mut SqliteConnection, link: &StoredExternalLink, now: i64) -> Result<(), StoreError> {
    let checked = link.state.is_some().then_some(now);
    sqlx::query(
        "INSERT INTO ticket_links_external \
         (ticket_id, system, connection_id, key, url, created_at, origin, close_on_done, title, state, checked_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&link.ticket_id)
    .bind(&link.system)
    .bind(&link.connection_id)
    .bind(&link.key)
    .bind(&link.url)
    .bind(now)
    .bind(link.origin.as_str())
    .bind(link.close_on_done)
    .bind(&link.title)
    .bind(link.state.map(|s| s.as_str()))
    .bind(checked)
    .execute(&mut *conn)
    .await?;
    Ok(())
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
        // The ticket came from the issue: Done closes it, as an import's link always has.
        let link = StoredExternalLink {
            ticket_id: id.to_string(),
            origin: LinkOrigin::Imported,
            close_on_done: true,
            ..link.clone()
        };
        insert_link(&mut tx, &link, now).await?;
        tx.commit().await?;
        Ok(Ok(self.get_ticket(id).await?.ok_or(TicketError::NotFound("ticket"))?))
    }

    /// Links ticket `link.ticket_id` to an external issue, on behalf of `by`: one it was filed as (PDT-221), or one
    /// someone linked by hand (PDT-481), as `link.origin` says. Its source is unchanged: the issue came from the ticket,
    /// or is only linked to it. `Ok(Err(id))` when a ticket already links that issue (nothing changes);
    /// NotFound("ticket") when the ticket is gone.
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
        insert_link(&mut tx, link, now).await?;
        touch(&mut tx, &link.ticket_id, now).await?;
        let data = json!({"system": link.system, "key": link.key, "url": link.url, "origin": link.origin.as_str()});
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

    /// A ticket's links to issues of `system`, those Done closes (or moves) first: a step given the ticket (closing
    /// its GitHub issue, moving its Jira issue) acts on the first, which is the one the Done built-ins mean when their
    /// trigger's `closing` names it (PDT-481).
    pub async fn external_links(&self, ticket_id: &str, system: &str) -> Result<Vec<StoredExternalLink>, StoreError> {
        let sql = format!("{STORED_LINK} WHERE ticket_id = ? AND system = ? ORDER BY close_on_done DESC, key");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(ticket_id).bind(system).fetch_all(&self.pool).await?;
        rows.iter().map(to_stored_link).collect()
    }

    /// Every link of a ticket to an issue, by system and key.
    pub async fn issue_links(&self, ticket_id: &str) -> Result<Vec<StoredExternalLink>, StoreError> {
        let sql = format!("{STORED_LINK} WHERE ticket_id = ? ORDER BY system, key");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(ticket_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_stored_link).collect()
    }

    /// A ticket's links that were last read before `before` (or never): the ones viewing it reads again.
    pub async fn issue_links_to_read(
        &self,
        ticket_id: &str,
        before: i64,
    ) -> Result<Vec<StoredExternalLink>, StoreError> {
        let sql = format!(
            "{STORED_LINK} WHERE ticket_id = ? AND (checked_at IS NULL OR checked_at <= ?) ORDER BY system, key"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(ticket_id).bind(before).fetch_all(&self.pool).await?;
        rows.iter().map(to_stored_link).collect()
    }

    /// Records what the tracker says of a linked issue now (its title and state). Not an edit of the ticket.
    pub async fn issue_seen(
        &self,
        link: &StoredExternalLink,
        title: &str,
        state: IssueState,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "UPDATE ticket_links_external SET title = ?, state = ?, checked_at = ? \
             WHERE system = ? AND connection_id = ? AND key = ?",
        )
        .bind(title)
        .bind(state.as_str())
        .bind(now)
        .bind(&link.system)
        .bind(&link.connection_id)
        .bind(&link.key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Marks a linked issue as read at `now` without news (the tracker couldn't be reached): it isn't tried again on
    /// every view.
    pub async fn issue_checked(&self, link: &StoredExternalLink, now: i64) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "UPDATE ticket_links_external SET checked_at = ? WHERE system = ? AND connection_id = ? AND key = ?",
        )
        .bind(now)
        .bind(&link.system)
        .bind(&link.connection_id)
        .bind(&link.key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Removes ticket `ticket_id`'s link to issue `key` of `system`, on behalf of `by` (an `unlinked` event).
    /// NotFound("link") when it has no such link; NotFound("ticket") when the ticket is gone.
    pub async fn unlink_external(
        &self,
        ticket_id: &str,
        system: &str,
        key: &str,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Ticket> {
        let mut tx = self.write_tx().await?;
        let gone: Option<(String,)> = sqlx::query_as(
            "DELETE FROM ticket_links_external WHERE ticket_id = ? AND system = ? AND key = ? RETURNING url",
        )
        .bind(ticket_id)
        .bind(system)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((url,)) = gone else {
            let exists: Option<(String,)> =
                sqlx::query_as("SELECT id FROM tickets WHERE id = ?").bind(ticket_id).fetch_optional(&mut *tx).await?;
            return Err(TicketError::NotFound(if exists.is_some() { "link" } else { "ticket" }));
        };
        touch(&mut tx, ticket_id, now).await?;
        let data = json!({"system": system, "key": key, "url": url});
        insert_event(&mut tx, ticket_id, EventKind::Unlinked, by, None, Some(data), now).await?;
        tx.commit().await?;
        self.get_ticket(ticket_id).await?.ok_or(TicketError::NotFound("ticket"))
    }

    /// Whether Done closes (or moves) the linked issue `key` of `system` (PDT-481: a manual link's toggle).
    /// NotFound("link") when the ticket has no such link.
    pub async fn set_close_on_done(
        &self,
        ticket_id: &str,
        system: &str,
        key: &str,
        on: bool,
        now: i64,
    ) -> TResult<Ticket> {
        let mut tx = self.write_tx().await?;
        let r = sqlx::query(
            "UPDATE ticket_links_external SET close_on_done = ? WHERE ticket_id = ? AND system = ? AND key = ?",
        )
        .bind(on)
        .bind(ticket_id)
        .bind(system)
        .bind(key)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 0 {
            return Err(TicketError::NotFound("link"));
        }
        touch(&mut tx, ticket_id, now).await?;
        tx.commit().await?;
        self.get_ticket(ticket_id).await?.ok_or(TicketError::NotFound("ticket"))
    }

    /// Adds a plain link (an address and its label, checked by the caller) to a ticket, on behalf of `by`. Adding an
    /// address it already has relabels it. NotFound("ticket") when the ticket is gone.
    pub async fn add_web_link(
        &self,
        ticket_id: &str,
        url: &str,
        label: &str,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Ticket> {
        let mut tx = self.write_tx().await?;
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM tickets WHERE id = ?").bind(ticket_id).fetch_optional(&mut *tx).await?;
        if exists.is_none() {
            return Err(TicketError::NotFound("ticket"));
        }
        sqlx::query(
            "INSERT INTO ticket_web_links (id, ticket_id, url, label, created_at) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (ticket_id, url) DO UPDATE SET label = excluded.label",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(ticket_id)
        .bind(url)
        .bind(label)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        touch(&mut tx, ticket_id, now).await?;
        let data = json!({"url": url, "label": label});
        insert_event(&mut tx, ticket_id, EventKind::Linked, by, None, Some(data), now).await?;
        tx.commit().await?;
        self.get_ticket(ticket_id).await?.ok_or(TicketError::NotFound("ticket"))
    }

    /// Removes a ticket's plain link `link_id`, on behalf of `by`. NotFound("link") when it has no such link.
    pub async fn remove_web_link(
        &self,
        ticket_id: &str,
        link_id: &str,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Ticket> {
        let mut tx = self.write_tx().await?;
        let gone: Option<(String, String)> =
            sqlx::query_as("DELETE FROM ticket_web_links WHERE ticket_id = ? AND id = ? RETURNING url, label")
                .bind(ticket_id)
                .bind(link_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((url, label)) = gone else { return Err(TicketError::NotFound("link")) };
        touch(&mut tx, ticket_id, now).await?;
        let data = json!({"url": url, "label": label});
        insert_event(&mut tx, ticket_id, EventKind::Unlinked, by, None, Some(data), now).await?;
        tx.commit().await?;
        self.get_ticket(ticket_id).await?.ok_or(TicketError::NotFound("ticket"))
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
