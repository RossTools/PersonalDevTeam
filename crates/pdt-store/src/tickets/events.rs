//! The timeline: events and comments.

use super::*;

pub(super) async fn insert_event(
    conn: &mut SqliteConnection,
    ticket_id: &str,
    kind: EventKind,
    by: impl Into<By>,
    body: Option<&str>,
    data: Option<serde_json::Value>,
    now: i64,
) -> Result<String, StoreError> {
    let by = by.into();
    let (actor, data) = (by.actor, by.stamp(data));
    let id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO ticket_events (id, ticket_id, kind, actor, body, data, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(ticket_id)
    .bind(kind.as_str())
    .bind(actor.as_str())
    .bind(body)
    .bind(data.as_ref().map(|d| d.to_string()))
    .bind(now)
    .execute(&mut *conn)
    .await?;
    let row = crate::event_log::TimelineRow { id: &id, ticket_id, kind, actor, body, data: data.as_ref() };
    crate::event_log::mirror_timeline(conn, row, now).await?;
    Ok(id)
}

pub(super) fn to_event(r: &SqliteRow) -> Result<TicketEvent, StoreError> {
    let kind: String = r.try_get("kind")?;
    let actor: String = r.try_get("actor")?;
    let data: Option<serde_json::Value> =
        r.try_get::<Option<String>, _>("data")?.and_then(|d| serde_json::from_str(&d).ok());
    let kind = EventKind::parse(&kind).ok_or_else(|| corrupt("event kind", &kind))?;
    let actor = Actor::parse(&actor).ok_or_else(|| corrupt("event actor", &actor))?;
    Ok(TicketEvent {
        id: r.try_get("id")?,
        ticket_id: r.try_get("ticket_id")?,
        kind,
        actor,
        body: r.try_get("body")?,
        agent: AgentBy::of_event(kind, actor, data.as_ref()),
        proposed_by: AgentBy::proposed(data.as_ref()),
        data,
        created_at: r.try_get("created_at")?,
    })
}

impl Store {
    /// The keys of the pull-request notices told to an attempt (`pr` events; untold ones do not count), so each is told once,
    /// across pdtd restarts too.
    pub async fn pr_notice_keys(&self, ticket_id: &str, attempt_id: &str) -> Result<HashSet<String>, StoreError> {
        let rows: Vec<(Option<String>,)> =
            sqlx::query_as("SELECT data FROM ticket_events WHERE ticket_id = ? AND kind = 'pr'")
                .bind(ticket_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(d,)| serde_json::from_str::<serde_json::Value>(&d?).ok())
            .filter(|v| v.get("attemptId").and_then(|a| a.as_str()) == Some(attempt_id))
            // A notice nobody was told does not spend its key (PDT-129).
            .filter(|v| v.get("told").and_then(|t| t.as_bool()) != Some(false))
            .filter_map(|v| v.get("key").and_then(|k| k.as_str()).map(String::from))
            .collect())
    }

    /// Adds an event to a ticket's timeline (e.g. `session` when Start made one).
    pub async fn add_ticket_event(
        &self,
        ticket_id: &str,
        kind: EventKind,
        by: impl Into<By>,
        body: Option<&str>,
        data: Option<serde_json::Value>,
        now: i64,
    ) -> Result<(), StoreError> {
        let body = body.map(|b| self.redactor.string(b));
        let data = data.map(|d| self.redactor.json(&d));
        let mut conn = self.pool.acquire().await?;
        insert_event(&mut conn, ticket_id, kind, by, body.as_deref(), data, now).await?;
        Ok(())
    }

    /// The data of the ticket's newest event of `kind` (None without one, or with no data).
    pub async fn latest_event_data(
        &self,
        ticket_id: &str,
        kind: EventKind,
    ) -> Result<Option<serde_json::Value>, StoreError> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT data FROM ticket_events WHERE ticket_id = ? AND kind = ? ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(ticket_id)
        .bind(kind.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(d,)| d).and_then(|d| serde_json::from_str(&d).ok()))
    }

    /// The text of the ticket's comments, oldest first (for Start's first prompt).
    pub async fn comment_bodies(&self, ticket_id: &str) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(Option<String>,)> =
            sqlx::query_as("SELECT body FROM ticket_events WHERE ticket_id = ? AND kind = ? ORDER BY created_at, id")
                .bind(ticket_id)
                .bind(EventKind::Comment.as_str())
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().filter_map(|(b,)| b).collect())
    }

    /// Whether an agent session created the ticket with `create_ticket` (its note says so, PDT-162).
    pub async fn created_by_session(&self, ticket_id: &str, session_id: &str) -> Result<bool, StoreError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM ticket_events WHERE ticket_id = ? AND kind = ? \
             AND COALESCE(json_extract(data, '$.agent.sessionId'), json_extract(data, '$.sessionId')) = ? AND json_extract(data, '$.created') = 1 LIMIT 1",
        )
        .bind(ticket_id)
        .bind(EventKind::Comment.as_str())
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// A ticket's events of `kinds`, oldest first (its comments, its history for `get_ticket`): the newest `limit`.
    pub async fn events_of(
        &self,
        ticket_id: &str,
        kinds: &[EventKind],
        limit: u32,
    ) -> Result<Vec<TicketEvent>, StoreError> {
        if kinds.is_empty() {
            return Ok(vec![]);
        }
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT id, ticket_id, kind, actor, body, data, created_at FROM ticket_events WHERE ticket_id = ",
        );
        qb.push_bind(ticket_id.to_string()).push(" AND kind IN (");
        let mut sep = qb.separated(", ");
        for k in kinds {
            sep.push_bind(k.as_str());
        }
        qb.push(") ORDER BY created_at DESC, id DESC LIMIT ").push_bind(limit as i64);
        let rows = qb.build().fetch_all(&self.pool).await?;
        let mut out = rows.iter().map(to_event).collect::<Result<Vec<_>, _>>()?;
        out.reverse();
        Ok(out)
    }

    /// The ticket's timeline, oldest first (or newest first), paged. None if there is no such ticket.
    pub async fn list_events(
        &self,
        ticket_id: &str,
        cursor: Option<&str>,
        limit: u32,
        newest_first: bool,
    ) -> TResult<Option<EventPage>> {
        let mut conn = self.pool.acquire().await?;
        if ticket_head(&mut conn, ticket_id).await?.is_none() {
            return Ok(None);
        }
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT id, ticket_id, kind, actor, body, data, created_at FROM ticket_events WHERE ticket_id = ",
        );
        qb.push_bind(ticket_id.to_string());
        if let Some(c) = cursor {
            let bad = || TicketError::Invalid("that cursor is not valid".into());
            let (ts, id) = c.split_once(':').ok_or_else(bad)?;
            let ts: i64 = ts.parse().map_err(|_| bad())?;
            qb.push(if newest_first { " AND (created_at, id) < (" } else { " AND (created_at, id) > (" })
                .push_bind(ts)
                .push(", ")
                .push_bind(id.to_string())
                .push(")");
        }
        qb.push(if newest_first { " ORDER BY created_at DESC, id DESC" } else { " ORDER BY created_at, id" });
        qb.push(" LIMIT ").push_bind(limit as i64 + 1);
        let rows = qb.build().fetch_all(&mut *conn).await?;
        let mut items = rows.iter().map(to_event).collect::<Result<Vec<_>, _>>()?;
        let more = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_cursor = if more { items.last().map(|e| format!("{}:{}", e.created_at, e.id)) } else { None };
        Ok(Some(EventPage { items, next_cursor }))
    }

    /// Adds a comment to the ticket's timeline. None if there is no such ticket.
    pub async fn add_comment(&self, ticket_id: &str, body: &str, now: i64) -> Result<Option<TicketEvent>, StoreError> {
        self.add_comment_by(ticket_id, body, Actor::User, None, now).await
    }

    /// The comment a workflow step wrote on the ticket (its `data.stepId`), if it did: a step run again after a restart
    /// finds it instead of commenting twice (PDT-131).
    pub async fn comment_by_step(&self, ticket_id: &str, step_id: &str) -> Result<Option<TicketEvent>, StoreError> {
        let row = sqlx::query(
            "SELECT id, ticket_id, kind, actor, body, data, created_at FROM ticket_events WHERE ticket_id = ? \
             AND kind = 'comment' AND json_extract(data, '$.stepId') = ? LIMIT 1",
        )
        .bind(ticket_id)
        .bind(step_id)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(to_event).transpose()
    }

    /// A comment by `by`; `data` names the workflow and run that wrote a system comment. Known secrets' values are
    /// redacted from its body.
    pub async fn add_comment_by(
        &self,
        ticket_id: &str,
        body: &str,
        by: impl Into<By>,
        data: Option<serde_json::Value>,
        now: i64,
    ) -> Result<Option<TicketEvent>, StoreError> {
        // A secret pasted into a comment is kept as its placeholder (PDT-158).
        let body = self.redactor.string(body);
        let mut tx = self.write_tx().await?;
        if ticket_head(&mut tx, ticket_id).await?.is_none() {
            return Ok(None);
        }
        let id = insert_event(&mut tx, ticket_id, EventKind::Comment, by, Some(&body), data, now).await?;
        touch(&mut tx, ticket_id, now).await?;
        let row =
            sqlx::query("SELECT id, ticket_id, kind, actor, body, data, created_at FROM ticket_events WHERE id = ?")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
        let event = to_event(&row)?;
        tx.commit().await?;
        Ok(Some(event))
    }
}
