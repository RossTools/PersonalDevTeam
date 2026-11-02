//! Making tickets and parts.

use super::*;

/// A part, for its parent's roll-up and "Start next part" (see `Store::part_rows`).
#[derive(Debug, Clone, PartialEq)]
pub struct PartRow {
    pub parent_id: String,
    pub id: String,
    /// `PDT-46`.
    pub key: String,
    pub title: String,
    pub stage: TicketStage,
    /// The active attempt, else the latest one: id and state.
    pub attempt: Option<(String, AttemptState)>,
    /// Open tickets blocking it.
    pub open_blockers: u32,
}

/// The facts `valid_parent` needs for making `child` (None: a new ticket) a part of `parent`.
pub(super) async fn parent_facts(
    conn: &mut SqliteConnection,
    child: Option<(&str, &str)>,
    parent: &str,
) -> TResult<(ParentFacts, String, String)> {
    let row: Option<(String, Option<String>, String, String, i64)> = sqlx::query_as(
        "SELECT t.project_id, t.parent_id, t.stage, COALESCE(p.key, ''), t.number FROM tickets t \
         JOIN projects p ON p.id = t.project_id WHERE t.id = ?",
    )
    .bind(parent)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((parent_project, grand, stage, pkey, number)) = row else {
        return Err(TicketError::Invalid("the parent ticket does not exist".into()));
    };
    let (child_id, child_project) = child.unwrap_or(("", &parent_project));
    let child_has_parts = if child_id.is_empty() {
        false
    } else {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tickets WHERE parent_id = ?")
            .bind(child_id)
            .fetch_one(&mut *conn)
            .await?;
        n > 0
    };
    let facts = ParentFacts {
        same_project: child_project == parent_project,
        is_self: child_id == parent,
        parent_has_parent: grand.is_some(),
        child_has_parts,
        parent_has_active_attempt: has_active_attempt(conn, parent).await?,
        parent_closed: stage_of(&stage)?.is_closed(),
    };
    Ok((facts, format_key(&pkey, u32::try_from(number).unwrap_or(0)), parent_project))
}

/// The parent's last part by rank (id), if it has any.
async fn last_part(conn: &mut SqliteConnection, parent: &str) -> Result<Option<String>, StoreError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT id FROM tickets WHERE parent_id = ? ORDER BY rank DESC, id DESC LIMIT 1")
            .bind(parent)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row.map(|(id,)| id))
}

/// `parted` events on a parent and a part.
pub(super) async fn parted_events(
    conn: &mut SqliteConnection,
    (parent, parent_key): (&str, &str),
    (part, part_key): (&str, &str),
    removed: bool,
    by: &By,
    now: i64,
) -> Result<(), StoreError> {
    let data =
        json!({"parentId": parent, "parentKey": parent_key, "partId": part, "partKey": part_key, "removed": removed});
    insert_event(conn, parent, EventKind::Parted, by, None, Some(data.clone()), now).await?;
    insert_event(conn, part, EventKind::Parted, by, None, Some(data), now).await?;
    Ok(())
}

/// How a ticket was made (PDT-366): with Make into parts and the agent's reason (both on its `created` event), and
/// whether it is held as a proposal for the user from the start, so no agent can move it on before its Inbox item
/// exists.
#[derive(Debug, Clone, Copy, Default)]
pub struct Made<'a> {
    pub via_parts: bool,
    pub reason: Option<&'a str>,
    pub proposal: bool,
}

/// Inserts a ticket with the project's next number (see `Store::create_ticket`). A part (`t.parent`) is checked
/// against the parent rules, goes after the parent's last part (or right after the parent) unless `before`/`after`
/// say otherwise, and with `blocked_by_previous` is blocked by that last part.
pub(super) async fn insert_ticket(
    conn: &mut SqliteConnection,
    id: &str,
    project_id: &str,
    t: &ValidNewTicket,
    by: &By,
    made: Made<'_>,
    now: i64,
) -> TResult<()> {
    let number: Option<(i64,)> = sqlx::query_as(
        "UPDATE projects SET next_ticket_number = next_ticket_number + 1 WHERE id = ? \
         RETURNING next_ticket_number - 1",
    )
    .bind(project_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((number,)) = number else { return Err(TicketError::NotFound("project")) };
    let mut previous = None;
    let mut parent_key = String::new();
    if let Some(parent) = &t.parent {
        let (facts, key, _) = parent_facts(conn, Some(("", project_id)), parent).await?;
        valid_parent(&facts)?;
        parent_key = key;
        previous = last_part(conn, parent).await?;
    }
    let place = if t.before.is_some() || t.after.is_some() {
        Place::Between { after: t.after.clone(), before: t.before.clone() }
    } else if let Some(parent) = &t.parent {
        Place::Between { after: Some(previous.clone().unwrap_or_else(|| parent.clone())), before: None }
    } else {
        Place::Edge(RankEdge::Top, t.stage)
    };
    let key = rank_for(conn, project_id, None, &place).await?;
    let closed_at = if t.stage.is_closed() { Some(now) } else { None };
    // An external ticket made on the board is let in only when the user makes it so (PDT-201).
    let boards = pdt_core::ticket::boards(None, Some(t.stage), &[], Some(&t.labels));
    let admitted = pdt_core::ticket::external_gate(t.source, false, boards, by, &t.title)
        .map_err(TicketError::Untrusted)?
        .then_some(now);
    sqlx::query(
        "INSERT INTO tickets (id, seq, project_id, number, title, description, stage, priority, rank, created_at, \
         updated_at, closed_at, parent_id, source, admitted_at, created_by, proposed_at) \
         VALUES (?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM tickets), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(project_id)
    .bind(number)
    .bind(&t.title)
    .bind(&t.description)
    .bind(t.stage.as_str())
    .bind(t.priority.as_str())
    .bind(&key)
    .bind(now)
    .bind(now)
    .bind(closed_at)
    .bind(&t.parent)
    .bind(t.source.as_str())
    .bind(admitted)
    .bind(by.agent.as_ref().and_then(|a| serde_json::to_string(a).ok()))
    .bind((made.proposal && t.stage == TicketStage::Draft).then_some(now))
    .execute(&mut *conn)
    .await?;
    replace_set(conn, "ticket_labels", "label", id, &t.labels).await?;
    replace_set(conn, "ticket_areas", "area", id, &t.areas).await?;
    if let (true, Some(parent)) = (t.areas.is_empty(), &t.parent) {
        // A part starts with its parent's areas (planning hints; overlap ignores parents themselves).
        sqlx::query("INSERT INTO ticket_areas (ticket_id, area) SELECT ?, area FROM ticket_areas WHERE ticket_id = ?")
            .bind(id)
            .bind(parent)
            .execute(&mut *conn)
            .await?;
    }
    if rank::needs_rebalance(&key) {
        rebalance(conn, project_id).await?;
    }
    let mut data = json!({"stage": t.stage});
    if made.via_parts {
        data["viaParts"] = json!(true);
    }
    if let Some(r) = made.reason.map(str::trim).filter(|r| !r.is_empty()) {
        data["reason"] = json!(r);
    }
    insert_event(conn, id, EventKind::Created, by, None, Some(data), now).await?;
    if let Some(parent) = &t.parent {
        let (pkey,): (String,) = sqlx::query_as("SELECT COALESCE(key, '') FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_one(&mut *conn)
            .await?;
        let own_key = format_key(&pkey, u32::try_from(number).unwrap_or(0));
        parted_events(conn, (parent, &parent_key), (id, &own_key), false, by, now).await?;
        if let (true, Some(prev)) = (t.blocked_by_previous, previous) {
            sqlx::query("INSERT INTO ticket_links (from_id, to_id, kind, created_at) VALUES (?, ?, 'blocks', ?)")
                .bind(&prev)
                .bind(id)
                .bind(now)
                .execute(&mut *conn)
                .await?;
            // A link is a change on both sides: auto-start's quiet period starts again (PDT-146).
            touch(conn, &prev, now).await?;
            let prev_key = ticket_head(conn, &prev).await?.map(|h| h.1).unwrap_or_default();
            link_events(conn, EventKind::Linked, LinkKind::Blocks, (&prev, &prev_key), (id, &own_key), by, now).await?;
        }
    }
    Ok(())
}

impl Store {
    /// Adds a ticket with the project's next number, at the top of its stage (or where `before`/`after` say), with a
    /// `created` event. NotFound("project") if there is no such project.
    pub async fn create_ticket(&self, id: &str, project_id: &str, t: &ValidNewTicket, now: i64) -> TResult<Ticket> {
        self.create_ticket_by(id, project_id, t, Actor::User, now).await
    }

    /// [`Store::create_ticket`] on behalf of `by` (the PM's `create_ticket`), who the `created` event names.
    pub async fn create_ticket_by(
        &self,
        id: &str,
        project_id: &str,
        t: &ValidNewTicket,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Ticket> {
        self.create_ticket_made(id, project_id, t, by, Made::default(), now).await
    }

    /// [`Store::create_ticket_by`] with the agent's `reason` for it and whether it is held as a proposal (PDT-366).
    pub async fn create_ticket_made(
        &self,
        id: &str,
        project_id: &str,
        t: &ValidNewTicket,
        by: impl Into<By>,
        made: Made<'_>,
        now: i64,
    ) -> TResult<Ticket> {
        let mut tx = self.write_tx().await?;
        insert_ticket(&mut tx, id, project_id, t, &by.into(), Made { via_parts: false, ..made }, now).await?;
        tx.commit().await?;
        self.get_ticket(id).await?.ok_or(TicketError::NotFound("ticket"))
    }

    /// Makes several parts of `parent`, in order after its existing parts, in one transaction (PDT-38). With
    /// `blocked_by_previous`, each is blocked by the part before it.
    pub async fn create_parts(&self, parent: &str, v: &ValidParts, now: i64) -> TResult<Vec<TicketSummary>> {
        self.create_parts_by(parent, v, TicketSource::User, Actor::User, now).await
    }

    /// [`Store::create_parts`] from `source` on behalf of `by`, who their `created` events name (with `viaParts`).
    pub async fn create_parts_by(
        &self,
        parent: &str,
        v: &ValidParts,
        source: TicketSource,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Vec<TicketSummary>> {
        let by = by.into();
        let mut tx = self.write_tx().await?;
        let Some((project_id, _, _)) = ticket_head(&mut tx, parent).await? else {
            return Err(TicketError::NotFound("ticket"));
        };
        let mut ids = vec![];
        for (title, description) in &v.parts {
            let id = uuid::Uuid::now_v7().to_string();
            let t = ValidNewTicket {
                title: title.clone(),
                description: description.clone(),
                stage: TicketStage::Draft,
                priority: Priority::None,
                labels: vec![],
                areas: vec![],
                before: None,
                after: None,
                parent: Some(parent.to_string()),
                blocked_by_previous: v.blocked_by_previous,
                source,
            };
            insert_ticket(&mut tx, &id, &project_id, &t, &by, Made { via_parts: true, ..Made::default() }, now).await?;
            ids.push(id);
        }
        tx.commit().await?;
        let mut out = vec![];
        for id in ids {
            out.extend(self.get_ticket_summary(&id).await?);
        }
        Ok(out)
    }

    /// The parts of each of `parents`, in rank order (PDT-38).
    pub async fn part_rows(&self, parents: &[String]) -> Result<Vec<PartRow>, StoreError> {
        let mut out = vec![];
        if parents.is_empty() {
            return Ok(out);
        }
        let mut conn = self.pool.acquire().await?;
        for chunk in parents.chunks(500) {
            let mut qb = QueryBuilder::<Sqlite>::new(format!(
                "SELECT t.parent_id, t.id, COALESCE(p.key, '') AS pkey, t.number, t.title, t.stage, \
                 {BLOCKED_BY} AS blocked_by, \
                 (SELECT a.id FROM attempts a WHERE a.ticket_id = t.id \
                  ORDER BY a.state = 'active' DESC, a.created_at DESC, a.id DESC LIMIT 1) AS attempt_id \
                 FROM tickets t JOIN projects p ON p.id = t.project_id WHERE t.parent_id IN "
            ));
            push_in(&mut qb, chunk);
            qb.push(" ORDER BY t.parent_id, t.rank, t.id");
            let rows = qb.build().fetch_all(&mut *conn).await?;
            let attempt_ids: Vec<String> =
                rows.iter().filter_map(|r| r.try_get::<Option<String>, _>("attempt_id").ok().flatten()).collect();
            let mut states: HashMap<String, AttemptState> = HashMap::new();
            for ids in attempt_ids.chunks(500) {
                let mut qb = QueryBuilder::<Sqlite>::new("SELECT id, state FROM attempts WHERE id IN ");
                push_in(&mut qb, ids);
                for r in qb.build().fetch_all(&mut *conn).await? {
                    let state: String = r.try_get("state")?;
                    states.insert(
                        r.try_get("id")?,
                        AttemptState::parse(&state).ok_or_else(|| corrupt("attempt state", &state))?,
                    );
                }
            }
            for r in &rows {
                let pkey: String = r.try_get("pkey")?;
                let attempt_id: Option<String> = r.try_get("attempt_id")?;
                out.push(PartRow {
                    parent_id: r.try_get("parent_id")?,
                    id: r.try_get("id")?,
                    key: format_key(&pkey, u32::try_from(r.try_get::<i64, _>("number")?).unwrap_or(0)),
                    title: r.try_get("title")?,
                    stage: stage_of(&r.try_get::<String, _>("stage")?)?,
                    attempt: attempt_id.and_then(|id| states.get(&id).map(|s| (id, *s))),
                    open_blockers: u32::try_from(r.try_get::<i64, _>("blocked_by")?).unwrap_or(0),
                });
            }
        }
        Ok(out)
    }

    /// A part started: its parent moves to In progress if it was Draft, Ready or To do (a system `moved` event).
    /// Returns whether it moved.
    pub async fn part_started(&self, parent: &str, now: i64) -> Result<bool, StoreError> {
        let mut tx = self.write_tx().await?;
        let Some((_, _, stage)) = ticket_head(&mut tx, parent).await? else { return Ok(false) };
        if !matches!(stage, TicketStage::Draft | TicketStage::Ready | TicketStage::Todo) {
            return Ok(false);
        }
        sqlx::query("UPDATE tickets SET stage = 'in_progress', updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(parent)
            .execute(&mut *tx)
            .await?;
        let data = json!({"from": stage, "to": TicketStage::InProgress});
        insert_event(&mut tx, parent, EventKind::Moved, Actor::System, None, Some(data), now).await?;
        tx.commit().await?;
        Ok(true)
    }
}
