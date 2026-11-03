//! Edits, stage changes, deletes and bulk actions.

use super::*;

/// A change to a ticket that workflows react to (PDT-363): `ticket.labels_changed`, `ticket.priority_changed` and
/// `ticket.parent_changed`. The parents are `(id, key)`.
enum Change {
    Labels { added: Vec<String>, removed: Vec<String> },
    Priority { from: Priority, to: Priority },
    Parent { from: Option<(String, String)>, to: Option<(String, String)> },
}

/// Logs `change` to ticket `id` by `by`, in the change's transaction (once per change: its key is new each time).
async fn log_change(
    conn: &mut SqliteConnection,
    id: &str,
    by: &By,
    change: Change,
    now: i64,
) -> Result<(), StoreError> {
    use pdt_core::workflow::{api::NewLogEvent, events as ev};
    let Some(t) = summary(conn, id).await? else { return Ok(()) };
    let ticket = ev::TicketChangeRef {
        project_id: t.project_id,
        ticket_id: id.into(),
        ticket_key: t.key,
        title: t.title,
        stage: t.stage.as_str().into(),
        actor: by.actor.as_str().into(),
        persona: by.agent.as_ref().and_then(|a| a.persona.clone()),
        session_id: by.agent.as_ref().map(|a| a.session_id.clone()),
    };
    let key = format!("{id}:{}", uuid::Uuid::now_v7());
    let e = match change {
        Change::Labels { added, removed } => {
            let p = ev::TicketLabelsChanged { ticket, added, removed, labels: t.labels };
            NewLogEvent::new(ev::TICKET_LABELS_CHANGED, key, "tickets", p)
        }
        Change::Priority { from, to } => {
            let p = ev::TicketPriorityChanged { ticket, from: from.as_str().into(), to: to.as_str().into() };
            NewLogEvent::new(ev::TICKET_PRIORITY_CHANGED, key, "tickets", p)
        }
        Change::Parent { from, to } => {
            let (from_id, from) = from.map_or((None, String::new()), |(i, k)| (Some(i), k));
            let (to_id, to) = to.map_or((None, String::new()), |(i, k)| (Some(i), k));
            let p = ev::TicketParentChanged { ticket, from, to, from_id, to_id };
            NewLogEvent::new(ev::TICKET_PARENT_CHANGED, key, "tickets", p)
        }
    };
    crate::event_log::emit_in(conn, &e, now).await?;
    Ok(())
}

/// Moves a ticket to `to` (closed_at follows), records the event, and notes tickets this one no longer blocks.
pub(super) async fn change_stage(
    conn: &mut SqliteConnection,
    id: &str,
    from: TicketStage,
    to: TicketStage,
    now: i64,
) -> Result<(), StoreError> {
    change_stage_by(conn, id, from, to, Actor::User, None, now).await
}

/// [`change_stage`] by `actor`, with `extra` added to the `moved` event's data (the workflow and run that did it).
async fn change_stage_by(
    conn: &mut SqliteConnection,
    id: &str,
    from: TicketStage,
    to: TicketStage,
    by: impl Into<By>,
    extra: Option<&serde_json::Value>,
    now: i64,
) -> Result<(), StoreError> {
    let closed_at = if to.is_closed() { Some(now) } else { None };
    // A closed ticket no longer waits to start; any move takes it out of its pipeline stage.
    sqlx::query(
        "UPDATE tickets SET stage = ?, updated_at = ?, pipeline_stage = NULL, \
         closed_at = CASE WHEN ? THEN COALESCE(closed_at, ?) ELSE NULL END, \
         waiting = CASE WHEN ? THEN 0 ELSE waiting END WHERE id = ?",
    )
    .bind(to.as_str())
    .bind(now)
    .bind(closed_at.is_some())
    .bind(now)
    .bind(closed_at.is_some())
    .bind(id)
    .execute(&mut *conn)
    .await?;
    if !matches!(to, TicketStage::Draft | TicketStage::Ready) {
        close_admit_items(conn, id, "admit", json!({"moved": to}), now).await?;
    }
    if to != TicketStage::Draft {
        // Decided: no longer held (PDT-366).
        sqlx::query("UPDATE tickets SET proposed_at = NULL WHERE id = ?").bind(id).execute(&mut *conn).await?;
        close_admit_items(conn, id, "ticket_proposal", json!({"moved": to}), now).await?;
    }
    let mut data = json!({"from": from, "to": to});
    if let (Some(d), Some(serde_json::Value::Object(x))) = (data.as_object_mut(), extra) {
        d.extend(x.clone());
    }
    insert_event(conn, id, EventKind::Moved, by, None, Some(data), now).await?;
    if to.is_closed() && !from.is_closed() {
        let sql = format!(
            "SELECT t.id FROM ticket_links l JOIN tickets t ON t.id = l.to_id \
             WHERE l.from_id = ? AND l.kind = 'blocks' AND {BLOCKED_BY} = 0"
        );
        let freed: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_all(&mut *conn).await?;
        for (t,) in freed {
            let data = json!({"by": id, "reason": "closed"});
            insert_event(conn, &t, EventKind::Unblocked, Actor::System, None, Some(data), now).await?;
        }
    }
    Ok(())
}

/// Closes a ticket's open "triaged ticket" Inbox items (`admit`, PDT-225: it moved past Ready) or "ticket proposal"
/// ones (`ticket_proposal`, PDT-366: it left Draft): nothing is left to decide.
async fn close_admit_items(
    conn: &mut SqliteConnection,
    id: &str,
    kind: &str,
    resolution: serde_json::Value,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE inbox_items SET status = 'closed', resolution = ?, resolved_at = ? \
         WHERE ticket_id = ? AND kind = ? AND status = 'open'",
    )
    .bind(resolution.to_string())
    .bind(now)
    .bind(id)
    .bind(kind)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// PDT-201's Todo gate on a change to `cur`: refused when it would put an external ticket the user hasn't let in in
/// front of agents and `by` isn't the user (or a call they confirmed); when it may, the ticket is let in. A ticket
/// proposal (PDT-366) refuses an agent's move to an open stage (`to`) or its `agentic` label.
async fn gate(
    conn: &mut SqliteConnection,
    cur: &TicketSummary,
    boards: bool,
    to: Option<TicketStage>,
    by: &By,
    now: i64,
) -> TResult<()> {
    let moves = to.is_some_and(|s| s != cur.stage && !s.is_closed());
    pdt_core::ticket_proposal::agent_gate(cur.proposed, by.actor, boards || moves, &cur.key)
        .map_err(TicketError::Untrusted)?;
    let admits = pdt_core::ticket::external_gate(cur.source, cur.admitted, boards, by, &cur.key)
        .map_err(TicketError::Untrusted)?;
    if admits {
        admit(conn, &cur.id, now).await?;
    }
    Ok(())
}

pub(super) async fn admit(conn: &mut SqliteConnection, id: &str, now: i64) -> Result<(), StoreError> {
    sqlx::query("UPDATE tickets SET admitted_at = COALESCE(admitted_at, ?) WHERE id = ? AND source = 'external'")
        .bind(now)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub(super) async fn replace_set(
    conn: &mut SqliteConnection,
    table: &str,
    col: &str,
    id: &str,
    values: &[String],
) -> Result<(), StoreError> {
    sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE ticket_id = ?")))
        .bind(id)
        .execute(&mut *conn)
        .await?;
    for v in values {
        sqlx::query(sqlx::AssertSqlSafe(format!("INSERT INTO {table} (ticket_id, {col}) VALUES (?, ?)")))
            .bind(id)
            .bind(v)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Deletes tickets and renumbers the tabs of the workspaces that showed them.
async fn delete_rows(conn: &mut SqliteConnection, ids: &[String]) -> Result<(), StoreError> {
    let mut workspaces: HashSet<String> = HashSet::new();
    for id in ids {
        let ws: Option<(String,)> = sqlx::query_as("SELECT workspace_id FROM workspace_tabs WHERE ticket_id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
        workspaces.extend(ws.map(|(w,)| w));
        // Nothing is left to approve (PDT-225) or decide (PDT-366).
        sqlx::query(
            "DELETE FROM inbox_items WHERE ticket_id = ? AND kind IN ('admit', 'ticket_proposal') AND status = 'open'",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM tickets WHERE id = ?").bind(id).execute(&mut *conn).await?;
    }
    for w in workspaces {
        crate::workspaces::renumber_tabs(conn, &w).await?;
    }
    Ok(())
}

/// The keys of a ticket's open parts, rank order (ignoring `except`).
async fn open_part_keys(
    conn: &mut SqliteConnection,
    parent: &str,
    except: &HashSet<String>,
) -> Result<Vec<String>, StoreError> {
    let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
        "SELECT t.id, COALESCE(p.key, ''), t.number, t.stage FROM tickets t JOIN projects p ON p.id = t.project_id \
         WHERE t.parent_id = ? ORDER BY t.rank, t.id",
    )
    .bind(parent)
    .fetch_all(&mut *conn)
    .await?;
    let mut list = vec![];
    for (id, k, n, st) in rows {
        if !except.contains(&id) {
            list.push((format_key(&k, u32::try_from(n).unwrap_or(0)), stage_of(&st)?));
        }
    }
    Ok(open_parts(list.iter().map(|(k, s)| (k.as_str(), *s))))
}

async fn has_parts(conn: &mut SqliteConnection, id: &str) -> Result<bool, StoreError> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM tickets WHERE parent_id = ?").bind(id).fetch_one(&mut *conn).await?;
    Ok(n > 0)
}

impl Store {
    /// Applies an edit, recording `moved` for a stage change and `edited` for the rest. None if there is no such
    /// ticket.
    pub async fn patch_ticket(&self, id: &str, p: &ValidPatch, now: i64) -> TResult<Option<Ticket>> {
        self.patch_ticket_by(id, p, Actor::User, now).await
    }

    /// [`Store::patch_ticket`] on behalf of `by` (an agent's `update_ticket` or `edit_ticket`), who its events name.
    pub async fn patch_ticket_by(
        &self,
        id: &str,
        p: &ValidPatch,
        by: impl Into<By>,
        now: i64,
    ) -> TResult<Option<Ticket>> {
        let by = by.into();
        let mut tx = self.write_tx().await?;
        let Some(cur) = summary(&mut tx, id).await? else { return Ok(None) };
        let boards = pdt_core::ticket::boards(Some(cur.stage), p.stage, &cur.labels, p.labels.as_deref());
        gate(&mut tx, &cur, boards, p.stage, &by, now).await?;
        let (description,): (String,) =
            sqlx::query_as("SELECT description FROM tickets WHERE id = ?").bind(id).fetch_one(&mut *tx).await?;
        let mut fields: Vec<&str> = vec![];
        let mut changes = vec![];
        if let Some(title) = p.title.as_ref().filter(|t| **t != cur.title) {
            sqlx::query("UPDATE tickets SET title = ? WHERE id = ?").bind(title).bind(id).execute(&mut *tx).await?;
            fields.push("title");
        }
        if let Some(d) = p.description.as_ref().filter(|d| **d != description) {
            sqlx::query("UPDATE tickets SET description = ? WHERE id = ?").bind(d).bind(id).execute(&mut *tx).await?;
            fields.push("description");
        }
        if let Some(pr) = p.priority.filter(|pr| *pr != cur.priority) {
            sqlx::query("UPDATE tickets SET priority = ? WHERE id = ?")
                .bind(pr.as_str())
                .bind(id)
                .execute(&mut *tx)
                .await?;
            fields.push("priority");
            changes.push(Change::Priority { from: cur.priority, to: pr });
        }
        if let Some(l) = p.labels.as_ref().filter(|l| **l != cur.labels) {
            replace_set(&mut tx, "ticket_labels", "label", id, l).await?;
            fields.push("labels");
            let (added, removed) = pdt_core::ticket::label_change(&cur.labels, l);
            changes.push(Change::Labels { added, removed });
        }
        if let Some(a) = p.areas.as_ref().filter(|a| **a != cur.areas) {
            replace_set(&mut tx, "ticket_areas", "area", id, a).await?;
            fields.push("areas");
        }
        if !fields.is_empty() {
            touch(&mut tx, id, now).await?;
            insert_event(&mut tx, id, EventKind::Edited, &by, None, Some(json!({"fields": fields})), now).await?;
        }
        if let Some(parent) = &p.parent {
            let current = cur.parent.as_ref().map(|x| x.id.as_str());
            if current != parent.as_deref() {
                if let Some(old) = &cur.parent {
                    parted_events(&mut tx, (&old.id, &old.key), (id, &cur.key), true, &by, now).await?;
                }
                let mut to = None;
                if let Some(new) = parent {
                    let (facts, key, _) = parent_facts(&mut tx, Some((id, &cur.project_id)), new).await?;
                    valid_parent(&facts)?;
                    parted_events(&mut tx, (new, &key), (id, &cur.key), false, &by, now).await?;
                    to = Some((new.clone(), key));
                }
                let from = cur.parent.as_ref().map(|p| (p.id.clone(), p.key.clone()));
                changes.push(Change::Parent { from, to });
                sqlx::query("UPDATE tickets SET parent_id = ?, updated_at = ? WHERE id = ?")
                    .bind(parent)
                    .bind(now)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let stage = p.stage.unwrap_or(cur.stage);
        if stage != cur.stage {
            if stage.is_closed() {
                let open = open_part_keys(&mut tx, id, &HashSet::new()).await?;
                if !open.is_empty() {
                    return Err(TicketError::PartsOpen { key: cur.key.clone(), open });
                }
            }
            change_stage_by(&mut tx, id, cur.stage, stage, &by, None, now).await?;
        }
        for change in changes {
            log_change(&mut tx, id, &by, change, now).await?;
        }
        if let Some(waiting) = p.waiting {
            if waiting && stage.is_closed() {
                return Err(TicketError::Invalid(format!("{} is closed, so it can't wait to start", cur.key)));
            }
            if waiting && has_active_attempt(&mut tx, id).await? {
                return Err(TicketError::Invalid(format!("{} has already started", cur.key)));
            }
            let (now_waiting,): (i64,) =
                sqlx::query_as("SELECT waiting FROM tickets WHERE id = ?").bind(id).fetch_one(&mut *tx).await?;
            if (now_waiting != 0) != waiting {
                sqlx::query("UPDATE tickets SET waiting = ?, updated_at = ? WHERE id = ?")
                    .bind(waiting)
                    .bind(now)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                insert_event(&mut tx, id, EventKind::Waiting, &by, None, Some(json!({"waiting": waiting})), now)
                    .await?;
                if waiting {
                    // A waiting ticket sits at the top of its stage.
                    let key = rank_for(&mut tx, &cur.project_id, Some(id), &Place::Edge(RankEdge::Top, stage)).await?;
                    set_rank(&mut tx, &cur.project_id, id, &key).await?;
                }
            }
        }
        tx.commit().await?;
        Ok(self.get_ticket(id).await?)
    }

    /// Deletes a ticket (its tab, links, labels and events go with it). Refused while it has an active attempt.
    pub async fn delete_ticket(&self, id: &str) -> Result<DeleteTicket, StoreError> {
        let mut tx = self.write_tx().await?;
        if ticket_head(&mut tx, id).await?.is_none() {
            return Ok(DeleteTicket::NotFound);
        }
        if has_active_attempt(&mut tx, id).await? {
            return Ok(DeleteTicket::Active);
        }
        if has_parts(&mut tx, id).await? {
            return Ok(DeleteTicket::HasParts);
        }
        delete_rows(&mut tx, &[id.to_string()]).await?;
        tx.commit().await?;
        Ok(DeleteTicket::Deleted)
    }

    /// Applies one action to every ticket in `ids`, in one transaction: all of them change or none do. Returns how
    /// many changed.
    pub async fn bulk_tickets(&self, ids: &[String], action: &ValidBulk, now: i64) -> TResult<u32> {
        let mut seen = HashSet::new();
        let ids: Vec<String> = ids.iter().filter(|id| seen.insert(id.as_str())).cloned().collect();
        let mut tx = self.write_tx().await?;
        let mut heads = Vec::with_capacity(ids.len());
        for id in &ids {
            let Some(h) = ticket_head(&mut tx, id).await? else { return Err(TicketError::NotFound("ticket")) };
            heads.push((id.clone(), h));
        }
        let mut count = 0u32;
        match action {
            ValidBulk::Stage(to) => {
                if to.is_closed() {
                    // A parent closes only when its parts are closed already or close with it.
                    let closing: HashSet<String> = ids.iter().cloned().collect();
                    for (id, (_, key, from)) in &heads {
                        if !from.is_closed() {
                            let open = open_part_keys(&mut tx, id, &closing).await?;
                            if !open.is_empty() {
                                return Err(TicketError::PartsOpen { key: key.clone(), open });
                            }
                        }
                    }
                }
                for (id, (_, _, from)) in &heads {
                    if pdt_core::ticket::boards(Some(*from), Some(*to), &[], None) {
                        admit(&mut tx, id, now).await?;
                    }
                    if from != to {
                        change_stage(&mut tx, id, *from, *to, now).await?;
                        count += 1;
                    }
                }
            }
            ValidBulk::Priority(pr) => {
                for (id, _) in &heads {
                    let (from,): (String,) = sqlx::query_as("SELECT priority FROM tickets WHERE id = ?")
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await?;
                    let r =
                        sqlx::query("UPDATE tickets SET priority = ?, updated_at = ? WHERE id = ? AND priority <> ?")
                            .bind(pr.as_str())
                            .bind(now)
                            .bind(id)
                            .bind(pr.as_str())
                            .execute(&mut *tx)
                            .await?;
                    if r.rows_affected() > 0 {
                        let data = json!({"fields": ["priority"]});
                        insert_event(&mut tx, id, EventKind::Edited, Actor::User, None, Some(data), now).await?;
                        let from = Priority::parse(&from).unwrap_or(Priority::None);
                        log_change(&mut tx, id, &Actor::User.into(), Change::Priority { from, to: *pr }, now).await?;
                        count += 1;
                    }
                }
            }
            ValidBulk::AddLabel(label) | ValidBulk::RemoveLabel(label) => {
                let add = matches!(action, ValidBulk::AddLabel(_));
                for (id, (_, key, _)) in &heads {
                    let r = if add {
                        sqlx::query("INSERT OR IGNORE INTO ticket_labels (ticket_id, label) VALUES (?, ?)")
                    } else {
                        sqlx::query("DELETE FROM ticket_labels WHERE ticket_id = ? AND label = ?")
                    }
                    .bind(id)
                    .bind(label)
                    .execute(&mut *tx)
                    .await?;
                    if r.rows_affected() == 0 {
                        continue;
                    }
                    if add && label == pdt_core::ticket::AGENTIC_LABEL {
                        admit(&mut tx, id, now).await?;
                    }
                    if add {
                        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ticket_labels WHERE ticket_id = ?")
                            .bind(id)
                            .fetch_one(&mut *tx)
                            .await?;
                        if n as usize > MAX_LABELS {
                            return Err(TicketError::Invalid(format!(
                                "{key} would have more than {MAX_LABELS} labels"
                            )));
                        }
                    }
                    touch(&mut tx, id, now).await?;
                    let data = json!({"fields": ["labels"]});
                    insert_event(&mut tx, id, EventKind::Edited, Actor::User, None, Some(data), now).await?;
                    let one = vec![label.clone()];
                    let change = if add {
                        Change::Labels { added: one, removed: vec![] }
                    } else {
                        Change::Labels { added: vec![], removed: one }
                    };
                    log_change(&mut tx, id, &Actor::User.into(), change, now).await?;
                    count += 1;
                }
            }
            ValidBulk::Top => {
                // Lowest first, each to the top of its stage: the selection ends up on top in its old order.
                let mut order: Vec<(String, String, TicketStage, String)> = vec![];
                for (id, (project, _, stage)) in &heads {
                    let (r,): (String,) =
                        sqlx::query_as("SELECT rank FROM tickets WHERE id = ?").bind(id).fetch_one(&mut *tx).await?;
                    order.push((r, id.clone(), *stage, project.clone()));
                }
                order.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
                for (_, id, stage, project) in order.iter().rev() {
                    let key = rank_for(&mut tx, project, Some(id), &Place::Edge(RankEdge::Top, *stage)).await?;
                    set_rank(&mut tx, project, id, &key).await?;
                    count += 1;
                }
            }
            ValidBulk::Delete => {
                for (id, (_, key, _)) in &heads {
                    if has_active_attempt(&mut tx, id).await? {
                        return Err(TicketError::Active(key.clone()));
                    }
                    let (left,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tickets WHERE parent_id = ?")
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await?;
                    if left > 0 {
                        let (gone,): (i64,) = {
                            let mut qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM tickets WHERE parent_id = ");
                            qb.push_bind(id.clone()).push(" AND id IN ");
                            push_in(&mut qb, &ids);
                            qb.build_query_as().fetch_one(&mut *tx).await?
                        };
                        if gone < left {
                            return Err(TicketError::HasParts(key.clone()));
                        }
                    }
                }
                delete_rows(&mut tx, &ids).await?;
                count = ids.len() as u32;
            }
        }
        tx.commit().await?;
        Ok(count)
    }

    /// Ends an active attempt as `state` (merged or abandoned) with its event. Returns its ticket's id, or None when
    /// there is no such active attempt (already ended, or unknown).
    /// Moves a ticket to `stage` on behalf of `actor` (a workflow: `extra` names it on the `moved` event). Closing a
    /// parent with open parts is refused as in [`Store::patch_ticket`]. None for an unknown ticket.
    pub async fn move_ticket_by(
        &self,
        id: &str,
        stage: TicketStage,
        by: impl Into<By>,
        extra: Option<&serde_json::Value>,
        now: i64,
    ) -> TResult<Option<Ticket>> {
        let by = by.into();
        let mut tx = self.write_tx().await?;
        let Some(cur) = summary(&mut tx, id).await? else { return Ok(None) };
        let boards = pdt_core::ticket::boards(Some(cur.stage), Some(stage), &[], None);
        gate(&mut tx, &cur, boards, Some(stage), &by, now).await?;
        if stage != cur.stage {
            if stage.is_closed() {
                let open = open_part_keys(&mut tx, id, &HashSet::new()).await?;
                if !open.is_empty() {
                    return Err(TicketError::PartsOpen { key: cur.key.clone(), open });
                }
            }
            change_stage_by(&mut tx, id, cur.stage, stage, &by, extra, now).await?;
        }
        tx.commit().await?;
        Ok(self.get_ticket(id).await?)
    }
}
