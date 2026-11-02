//! Backlog order: rank keys and moves.

use super::*;

/// A project's tickets in backlog order: (id, rank, stage).
async fn ranked(conn: &mut SqliteConnection, project_id: &str) -> Result<Vec<(String, String, String)>, StoreError> {
    Ok(sqlx::query_as("SELECT id, rank, stage FROM tickets WHERE project_id = ? ORDER BY rank, id")
        .bind(project_id)
        .fetch_all(&mut *conn)
        .await?)
}

/// Rewrites a project's ranks evenly, keeping their order.
pub(super) async fn rebalance(conn: &mut SqliteConnection, project_id: &str) -> Result<(), StoreError> {
    let list = ranked(conn, project_id).await?;
    for ((id, _, _), key) in list.iter().zip(rank::spread(list.len())) {
        sqlx::query("UPDATE tickets SET rank = ? WHERE id = ?").bind(key).bind(id).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Where a ticket goes in its project's backlog.
#[derive(Debug, Clone)]
pub(super) enum Place {
    /// Directly below `after` and above `before` (either may be absent, not both).
    Between {
        after: Option<String>,
        before: Option<String>,
    },
    Edge(RankEdge, TicketStage),
}

/// A rank key for `place` among the project's tickets other than `moving`. Rebalances first when the neighbours
/// leave no room (duplicate keys).
pub(super) async fn rank_for(
    conn: &mut SqliteConnection,
    project_id: &str,
    moving: Option<&str>,
    place: &Place,
) -> TResult<String> {
    for attempt in 0..2 {
        let list: Vec<(String, String, String)> =
            ranked(conn, project_id).await?.into_iter().filter(|(id, _, _)| Some(id.as_str()) != moving).collect();
        let pos = |id: &str| {
            list.iter()
                .position(|(x, _, _)| x == id)
                .ok_or_else(|| TicketError::Invalid("before/after must name another ticket in the same project".into()))
        };
        let key = |i: usize| list[i].1.as_str();
        let (lo, hi): (Option<&str>, Option<&str>) = match place {
            Place::Between { after: Some(a), before: Some(b) } => {
                let (ia, ib) = (pos(a)?, pos(b)?);
                if ia >= ib {
                    return Err(TicketError::Invalid("`after` must come before `before` in the backlog".into()));
                }
                (Some(key(ia)), Some(key(ib)))
            }
            Place::Between { after: None, before: Some(b) } => {
                let ib = pos(b)?;
                (ib.checked_sub(1).map(key), Some(key(ib)))
            }
            Place::Between { after: Some(a), before: None } => {
                let ia = pos(a)?;
                (Some(key(ia)), list.get(ia + 1).map(|x| x.1.as_str()))
            }
            Place::Between { after: None, before: None } => (list.last().map(|x| x.1.as_str()), None),
            Place::Edge(edge, stage) => {
                let in_stage: Vec<usize> =
                    list.iter().enumerate().filter(|(_, x)| x.2 == stage.as_str()).map(|(i, _)| i).collect();
                match (edge, in_stage.first(), in_stage.last()) {
                    (RankEdge::Top, Some(&i), _) => (i.checked_sub(1).map(key), Some(key(i))),
                    (RankEdge::Bottom, _, Some(&i)) => (Some(key(i)), list.get(i + 1).map(|x| x.1.as_str())),
                    (RankEdge::Top, None, _) => (None, list.first().map(|x| x.1.as_str())),
                    (RankEdge::Bottom, _, None) => (list.last().map(|x| x.1.as_str()), None),
                }
            }
        };
        match rank::between(lo, hi) {
            Ok(k) => return Ok(k),
            // Neighbours with equal keys leave no room: spread the project's keys and try again.
            Err(_) if attempt == 0 => rebalance(conn, project_id).await?,
            Err(e) => return Err(TicketError::Store(StoreError::Corrupt(e.to_string()))),
        }
    }
    unreachable!("the loop returns")
}

/// Sets a ticket's rank, rebalancing the project when the key has grown too long.
pub(super) async fn set_rank(
    conn: &mut SqliteConnection,
    project_id: &str,
    id: &str,
    key: &str,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE tickets SET rank = ? WHERE id = ?").bind(key).bind(id).execute(&mut *conn).await?;
    if rank::needs_rebalance(key) {
        rebalance(conn, project_id).await?;
    }
    Ok(())
}

impl Store {
    /// Moves a ticket in its project's backlog: directly above `before`, below `after`, between them, or to the top or
    /// bottom of its stage. None if there is no such ticket.
    pub async fn move_ticket(
        &self,
        id: &str,
        before: Option<&str>,
        after: Option<&str>,
        to: Option<RankEdge>,
    ) -> TResult<Option<Ticket>> {
        let place = match (before, after, to) {
            // The stage is filled in once the ticket is loaded.
            (None, None, Some(edge)) => Place::Edge(edge, TicketStage::Draft),
            (b, a, None) if b.is_some() || a.is_some() => {
                Place::Between { after: a.map(String::from), before: b.map(String::from) }
            }
            _ => return Err(TicketError::Invalid("give `before` and/or `after`, or `to` (top or bottom)".into())),
        };
        if before == Some(id) || after == Some(id) {
            return Err(TicketError::Invalid("a ticket can't be placed next to itself".into()));
        }
        let mut tx = self.write_tx().await?;
        let Some((project_id, _, stage)) = ticket_head(&mut tx, id).await? else { return Ok(None) };
        let place = match place {
            Place::Edge(edge, _) => Place::Edge(edge, stage),
            other => other,
        };
        let key = rank_for(&mut tx, &project_id, Some(id), &place).await?;
        set_rank(&mut tx, &project_id, id, &key).await?;
        tx.commit().await?;
        Ok(self.get_ticket(id).await?)
    }
}
