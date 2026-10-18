//! Tickets (tickets spec §3, §4): numbering, ranks, the backlog query (filters, full-text search, keyset paging),
//! edits with their timeline events, bulk actions, links and comments.

use crate::{Store, StoreError};
use pdt_core::links::would_cycle;
use pdt_core::rank;
use pdt_core::ticket::{
    Actor, AttemptState, AttemptSummary, EventKind, EventPage, LinkDirection, LinkKind, LinkedTicket, MAX_LABELS,
    OverlapLevel, Priority, RankEdge, Ticket, TicketEvent, TicketFilter, TicketPage, TicketSort, TicketStage,
    TicketSummary, ValidBulk, ValidNewTicket, ValidPatch, format_key, parse_key,
};
use serde_json::json;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use std::collections::{HashMap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum TicketError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    /// The keys around the cycle, starting and ending with the ticket being linked from.
    #[error("that link would close a cycle: {}", .0.join(" → "))]
    Cycle(Vec<String>),
    /// The ticket (by key) has an active attempt.
    #[error("{0} has an active attempt")]
    Active(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<sqlx::Error> for TicketError {
    fn from(e: sqlx::Error) -> Self {
        TicketError::Store(e.into())
    }
}

type TResult<T> = Result<T, TicketError>;

/// Why a ticket was not deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteTicket {
    Deleted,
    NotFound,
    /// It has an active attempt.
    Active,
}

/// Priority as a number (higher is more urgent), for sorting.
const PRIORITY_WEIGHT: &str = "(CASE t.priority WHEN 'urgent' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 \
                               WHEN 'low' THEN 1 ELSE 0 END)";
/// How many open tickets block `t`.
const BLOCKED_BY: &str = "(SELECT COUNT(*) FROM ticket_links l JOIN tickets b ON b.id = l.from_id \
                          WHERE l.to_id = t.id AND l.kind = 'blocks' AND b.stage NOT IN ('done', 'wont_do'))";
const FROM: &str = " FROM tickets t JOIN projects p ON p.id = t.project_id";

fn summary_select() -> String {
    format!(
        "SELECT t.id, t.project_id, COALESCE(p.key, '') AS pkey, t.number, t.title, t.stage, t.priority, t.rank, \
         t.waiting, t.created_at, t.updated_at, t.closed_at, {BLOCKED_BY} AS blocked_by, {PRIORITY_WEIGHT} AS pw{FROM}"
    )
}

fn corrupt(what: &str, v: &str) -> StoreError {
    StoreError::Corrupt(format!("{what} {v:?}"))
}

fn stage_of(s: &str) -> Result<TicketStage, StoreError> {
    TicketStage::parse(s).ok_or_else(|| corrupt("ticket stage", s))
}

fn to_summary(r: &SqliteRow) -> Result<TicketSummary, StoreError> {
    let pkey: String = r.try_get("pkey")?;
    let number = u32::try_from(r.try_get::<i64, _>("number")?).unwrap_or(0);
    let priority: String = r.try_get("priority")?;
    Ok(TicketSummary {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        key: format_key(&pkey, number),
        number,
        title: r.try_get("title")?,
        stage: stage_of(&r.try_get::<String, _>("stage")?)?,
        priority: Priority::parse(&priority).ok_or_else(|| corrupt("ticket priority", &priority))?,
        labels: vec![],
        areas: vec![],
        rank: r.try_get("rank")?,
        waiting: r.try_get::<i64, _>("waiting")? != 0,
        blocked_by: u32::try_from(r.try_get::<i64, _>("blocked_by")?).unwrap_or(0),
        attempt: None,
        overlap: OverlapLevel::None,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        closed_at: r.try_get("closed_at")?,
    })
}

fn to_attempt(r: &SqliteRow) -> Result<AttemptSummary, StoreError> {
    let state: String = r.try_get("state")?;
    Ok(AttemptSummary {
        id: r.try_get("id")?,
        branch: r.try_get("branch")?,
        base_branch: r.try_get("base_branch")?,
        worktree_id: r.try_get("worktree_id")?,
        state: AttemptState::parse(&state).ok_or_else(|| corrupt("attempt state", &state))?,
        created_at: r.try_get("created_at")?,
        ended_at: r.try_get("ended_at")?,
    })
}

fn push_in(qb: &mut QueryBuilder<Sqlite>, values: &[String]) {
    qb.push("(");
    let mut sep = qb.separated(", ");
    for v in values {
        sep.push_bind(v.clone());
    }
    qb.push(")");
}

/// Fills in labels, areas and the attempt of each summary.
async fn decorate(conn: &mut SqliteConnection, items: &mut [TicketSummary]) -> Result<(), StoreError> {
    if items.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = items.iter().map(|t| t.id.clone()).collect();
    let mut labels: HashMap<String, Vec<String>> = HashMap::new();
    let mut areas: HashMap<String, Vec<String>> = HashMap::new();
    // Chunked so a large page never nears SQLite's bound-variable limit.
    for chunk in ids.chunks(500) {
        let mut qb =
            QueryBuilder::<Sqlite>::new("SELECT ticket_id, label, 0 AS area FROM ticket_labels WHERE ticket_id IN ");
        push_in(&mut qb, chunk);
        qb.push(" UNION ALL SELECT ticket_id, area, 1 FROM ticket_areas WHERE ticket_id IN ");
        push_in(&mut qb, chunk);
        qb.push(" ORDER BY 1, 3, 2");
        for r in qb.build().fetch_all(&mut *conn).await? {
            let (id, v, is_area): (String, String, i64) = (r.try_get(0)?, r.try_get(1)?, r.try_get(2)?);
            if is_area == 0 { labels.entry(id).or_default().push(v) } else { areas.entry(id).or_default().push(v) }
        }
    }
    let mut attempts: HashMap<String, AttemptSummary> = HashMap::new();
    for chunk in ids.chunks(500) {
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT id, ticket_id, branch, base_branch, worktree_id, state, created_at, ended_at FROM attempts \
             WHERE ticket_id IN ",
        );
        push_in(&mut qb, chunk);
        qb.push(" ORDER BY ticket_id, state = 'active' DESC, created_at DESC, id DESC");
        for r in qb.build().fetch_all(&mut *conn).await? {
            let tid: String = r.try_get("ticket_id")?;
            if let std::collections::hash_map::Entry::Vacant(e) = attempts.entry(tid) {
                e.insert(to_attempt(&r)?);
            }
        }
    }
    for t in items.iter_mut() {
        t.labels = labels.remove(&t.id).unwrap_or_default();
        t.areas = areas.remove(&t.id).unwrap_or_default();
        t.attempt = attempts.remove(&t.id);
    }
    Ok(())
}

async fn summary(conn: &mut SqliteConnection, id: &str) -> Result<Option<TicketSummary>, StoreError> {
    let sql = format!("{} WHERE t.id = ?", summary_select());
    let Some(row) = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&mut *conn).await? else {
        return Ok(None);
    };
    let mut items = [to_summary(&row)?];
    decorate(conn, &mut items).await?;
    let [s] = items;
    Ok(Some(s))
}

/// The FTS5 query for `q`: every word, prefix-matched (`"fix"* "log"*`). None when `q` has no words.
fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> =
        q.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| format!("\"{w}\"*")).collect();
    if words.is_empty() { None } else { Some(words.join(" ")) }
}

fn push_filters(qb: &mut QueryBuilder<Sqlite>, f: &TicketFilter) {
    qb.push(" WHERE 1 = 1");
    if let Some(p) = &f.project {
        qb.push(" AND t.project_id = ").push_bind(p.clone());
    }
    if !f.stages.is_empty() {
        qb.push(" AND t.stage IN ");
        push_in(qb, &f.stages.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>());
    }
    if !f.priorities.is_empty() {
        qb.push(" AND t.priority IN ");
        push_in(qb, &f.priorities.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>());
    }
    for l in &f.labels {
        qb.push(" AND EXISTS (SELECT 1 FROM ticket_labels tl WHERE tl.ticket_id = t.id AND tl.label = ")
            .push_bind(l.clone())
            .push(")");
    }
    match f.blocked {
        Some(true) => {
            qb.push(format!(" AND {BLOCKED_BY} > 0"));
        }
        Some(false) => {
            qb.push(format!(" AND {BLOCKED_BY} = 0"));
        }
        None => {}
    }
    if let Some(q) = &f.q {
        let fts = fts_query(q);
        let key = parse_key(q);
        qb.push(" AND (0");
        if let Some(fts) = fts {
            qb.push(" OR t.seq IN (SELECT rowid FROM tickets_fts WHERE tickets_fts MATCH ").push_bind(fts).push(")");
        }
        if let Some(k) = key {
            qb.push(" OR (t.number = ").push_bind(k.number as i64);
            if let Some(pk) = k.key {
                qb.push(" AND p.key = ").push_bind(pk);
            }
            qb.push(")");
        }
        qb.push(")");
    }
}

/// Keyset cursors: the sort, then the last row's sort values and id.
fn encode_cursor(sort: TicketSort, t: &TicketSummary) -> String {
    match sort {
        TicketSort::Rank => format!("r:{}:{}", t.rank, t.id),
        TicketSort::Priority => format!("p:{}:{}:{}", t.priority.weight(), t.rank, t.id),
        TicketSort::Updated => format!("u:{}:{}", t.updated_at, t.id),
        TicketSort::Created => format!("c:{}:{}", t.created_at, t.id),
    }
}

fn push_cursor(qb: &mut QueryBuilder<Sqlite>, sort: TicketSort, cursor: &str) -> TResult<()> {
    let bad = || TicketError::Invalid("that cursor is not valid for this sort".into());
    let num = |s: &str| s.parse::<i64>().map_err(|_| bad());
    match sort {
        TicketSort::Rank => {
            let mut it = cursor.strip_prefix("r:").ok_or_else(bad)?.splitn(2, ':');
            let (r, id) = (it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?);
            qb.push(" AND (t.rank, t.id) > (").push_bind(r.to_string()).push(", ").push_bind(id.to_string()).push(")");
        }
        TicketSort::Priority => {
            let mut it = cursor.strip_prefix("p:").ok_or_else(bad)?.splitn(3, ':');
            let (w, r, id) = (num(it.next().ok_or_else(bad)?)?, it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?);
            qb.push(format!(" AND ({PRIORITY_WEIGHT} < "))
                .push_bind(w)
                .push(format!(" OR ({PRIORITY_WEIGHT} = "))
                .push_bind(w)
                .push(" AND (t.rank, t.id) > (")
                .push_bind(r.to_string())
                .push(", ")
                .push_bind(id.to_string())
                .push(")))");
        }
        TicketSort::Updated | TicketSort::Created => {
            let (prefix, col) = if sort == TicketSort::Updated { ("u:", "updated_at") } else { ("c:", "created_at") };
            let mut it = cursor.strip_prefix(prefix).ok_or_else(bad)?.splitn(2, ':');
            let (ts, id) = (num(it.next().ok_or_else(bad)?)?, it.next().ok_or_else(bad)?);
            qb.push(format!(" AND (t.{col}, t.id) < (")).push_bind(ts).push(", ").push_bind(id.to_string()).push(")");
        }
    }
    Ok(())
}

fn order_by(sort: TicketSort) -> String {
    match sort {
        TicketSort::Rank => " ORDER BY t.rank, t.id".into(),
        TicketSort::Priority => format!(" ORDER BY {PRIORITY_WEIGHT} DESC, t.rank, t.id"),
        TicketSort::Updated => " ORDER BY t.updated_at DESC, t.id DESC".into(),
        TicketSort::Created => " ORDER BY t.created_at DESC, t.id DESC".into(),
    }
}

async fn insert_event(
    conn: &mut SqliteConnection,
    ticket_id: &str,
    kind: EventKind,
    actor: Actor,
    body: Option<&str>,
    data: Option<serde_json::Value>,
    now: i64,
) -> Result<String, StoreError> {
    let id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO ticket_events (id, ticket_id, kind, actor, body, data, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(ticket_id)
    .bind(kind.as_str())
    .bind(actor.as_str())
    .bind(body)
    .bind(data.map(|d| d.to_string()))
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

fn to_event(r: &SqliteRow) -> Result<TicketEvent, StoreError> {
    let kind: String = r.try_get("kind")?;
    let actor: String = r.try_get("actor")?;
    let data: Option<String> = r.try_get("data")?;
    Ok(TicketEvent {
        id: r.try_get("id")?,
        ticket_id: r.try_get("ticket_id")?,
        kind: EventKind::parse(&kind).ok_or_else(|| corrupt("event kind", &kind))?,
        actor: Actor::parse(&actor).ok_or_else(|| corrupt("event actor", &actor))?,
        body: r.try_get("body")?,
        data: data.and_then(|d| serde_json::from_str(&d).ok()),
        created_at: r.try_get("created_at")?,
    })
}

/// A project's tickets in backlog order: (id, rank, stage).
async fn ranked(conn: &mut SqliteConnection, project_id: &str) -> Result<Vec<(String, String, String)>, StoreError> {
    Ok(sqlx::query_as("SELECT id, rank, stage FROM tickets WHERE project_id = ? ORDER BY rank, id")
        .bind(project_id)
        .fetch_all(&mut *conn)
        .await?)
}

/// Rewrites a project's ranks evenly, keeping their order.
async fn rebalance(conn: &mut SqliteConnection, project_id: &str) -> Result<(), StoreError> {
    let list = ranked(conn, project_id).await?;
    for ((id, _, _), key) in list.iter().zip(rank::spread(list.len())) {
        sqlx::query("UPDATE tickets SET rank = ? WHERE id = ?").bind(key).bind(id).execute(&mut *conn).await?;
    }
    Ok(())
}

/// Where a ticket goes in its project's backlog.
#[derive(Debug, Clone)]
enum Place {
    /// Directly below `after` and above `before` (either may be absent, not both).
    Between {
        after: Option<String>,
        before: Option<String>,
    },
    Edge(RankEdge, TicketStage),
}

/// A rank key for `place` among the project's tickets other than `moving`. Rebalances first when the neighbours
/// leave no room (duplicate keys).
async fn rank_for(
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
async fn set_rank(conn: &mut SqliteConnection, project_id: &str, id: &str, key: &str) -> Result<(), StoreError> {
    sqlx::query("UPDATE tickets SET rank = ? WHERE id = ?").bind(key).bind(id).execute(&mut *conn).await?;
    if rank::needs_rebalance(key) {
        rebalance(conn, project_id).await?;
    }
    Ok(())
}

/// (project_id, key, stage) of a ticket.
async fn ticket_head(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<(String, String, TicketStage)>, StoreError> {
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT t.project_id, COALESCE(p.key, ''), t.number, t.stage FROM tickets t \
         JOIN projects p ON p.id = t.project_id WHERE t.id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|(p, k, n, s)| Ok((p, format_key(&k, u32::try_from(n).unwrap_or(0)), stage_of(&s)?))).transpose()
}

async fn has_active_attempt(conn: &mut SqliteConnection, id: &str) -> Result<bool, StoreError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT id FROM attempts WHERE ticket_id = ? AND state = 'active'")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row.is_some())
}

/// Moves a ticket to `to` (closed_at follows), records the event, and notes tickets this one no longer blocks.
async fn change_stage(
    conn: &mut SqliteConnection,
    id: &str,
    from: TicketStage,
    to: TicketStage,
    now: i64,
) -> Result<(), StoreError> {
    let closed_at = if to.is_closed() { Some(now) } else { None };
    sqlx::query(
        "UPDATE tickets SET stage = ?, updated_at = ?, \
         closed_at = CASE WHEN ? THEN COALESCE(closed_at, ?) ELSE NULL END WHERE id = ?",
    )
    .bind(to.as_str())
    .bind(now)
    .bind(closed_at.is_some())
    .bind(now)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    insert_event(conn, id, EventKind::Moved, Actor::User, None, Some(json!({"from": from, "to": to})), now).await?;
    if to.is_closed() && !from.is_closed() {
        let sql = format!(
            "SELECT t.id FROM ticket_links l JOIN tickets t ON t.id = l.to_id \
             WHERE l.from_id = ? AND l.kind = 'blocks' AND {BLOCKED_BY} = 0"
        );
        let freed: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_all(&mut *conn).await?;
        for (t,) in freed {
            insert_event(conn, &t, EventKind::Unblocked, Actor::System, None, Some(json!({"by": id})), now).await?;
        }
    }
    Ok(())
}

async fn touch(conn: &mut SqliteConnection, id: &str, now: i64) -> Result<(), StoreError> {
    sqlx::query("UPDATE tickets SET updated_at = ? WHERE id = ?").bind(now).bind(id).execute(&mut *conn).await?;
    Ok(())
}

async fn replace_set(
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
        sqlx::query("DELETE FROM tickets WHERE id = ?").bind(id).execute(&mut *conn).await?;
    }
    for w in workspaces {
        crate::workspaces::renumber_tabs(conn, &w).await?;
    }
    Ok(())
}

impl Store {
    /// Adds a ticket with the project's next number, at the top of its stage (or where `before`/`after` say), with a
    /// `created` event. NotFound("project") if there is no such project.
    pub async fn create_ticket(&self, id: &str, project_id: &str, t: &ValidNewTicket, now: i64) -> TResult<Ticket> {
        let mut tx = self.pool.begin().await?;
        let number: Option<(i64,)> = sqlx::query_as(
            "UPDATE projects SET next_ticket_number = next_ticket_number + 1 WHERE id = ? \
             RETURNING next_ticket_number - 1",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((number,)) = number else { return Err(TicketError::NotFound("project")) };
        let place = if t.before.is_some() || t.after.is_some() {
            Place::Between { after: t.after.clone(), before: t.before.clone() }
        } else {
            Place::Edge(RankEdge::Top, t.stage)
        };
        let key = rank_for(&mut tx, project_id, None, &place).await?;
        let closed_at = if t.stage.is_closed() { Some(now) } else { None };
        sqlx::query(
            "INSERT INTO tickets (id, seq, project_id, number, title, description, stage, priority, rank, created_at, \
             updated_at, closed_at) \
             VALUES (?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM tickets), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .execute(&mut *tx)
        .await?;
        replace_set(&mut tx, "ticket_labels", "label", id, &t.labels).await?;
        replace_set(&mut tx, "ticket_areas", "area", id, &t.areas).await?;
        if rank::needs_rebalance(&key) {
            rebalance(&mut tx, project_id).await?;
        }
        insert_event(&mut tx, id, EventKind::Created, Actor::User, None, Some(json!({"stage": t.stage})), now).await?;
        tx.commit().await?;
        self.get_ticket(id).await?.ok_or(TicketError::NotFound("ticket"))
    }

    pub async fn get_ticket_summary(&self, id: &str) -> Result<Option<TicketSummary>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        summary(&mut conn, id).await
    }

    /// The full ticket: description, links (with the other ticket's key, title and stage) and attempts.
    pub async fn get_ticket(&self, id: &str) -> Result<Option<Ticket>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        let Some(s) = summary(&mut conn, id).await? else { return Ok(None) };
        let (description,): (String,) =
            sqlx::query_as("SELECT description FROM tickets WHERE id = ?").bind(id).fetch_one(&mut *conn).await?;
        let rows = sqlx::query(
            "SELECT l.kind, 'outgoing' AS dir, o.id, COALESCE(p.key, '') AS pkey, o.number, o.title, o.stage \
             FROM ticket_links l JOIN tickets o ON o.id = l.to_id JOIN projects p ON p.id = o.project_id \
             WHERE l.from_id = ?1 \
             UNION ALL \
             SELECT l.kind, 'incoming', o.id, COALESCE(p.key, ''), o.number, o.title, o.stage \
             FROM ticket_links l JOIN tickets o ON o.id = l.from_id JOIN projects p ON p.id = o.project_id \
             WHERE l.to_id = ?1 \
             ORDER BY 1, 2, 5",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        let links = rows
            .iter()
            .map(|r| {
                let kind: String = r.try_get(0)?;
                let dir: String = r.try_get(1)?;
                let pkey: String = r.try_get(3)?;
                Ok(LinkedTicket {
                    kind: LinkKind::parse(&kind).ok_or_else(|| corrupt("link kind", &kind))?,
                    direction: if dir == "outgoing" { LinkDirection::Outgoing } else { LinkDirection::Incoming },
                    id: r.try_get(2)?,
                    key: format_key(&pkey, u32::try_from(r.try_get::<i64, _>(4)?).unwrap_or(0)),
                    title: r.try_get(5)?,
                    stage: stage_of(&r.try_get::<String, _>(6)?)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let attempts = sqlx::query(
            "SELECT id, ticket_id, branch, base_branch, worktree_id, state, created_at, ended_at FROM attempts \
             WHERE ticket_id = ? ORDER BY created_at DESC, id DESC",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(to_attempt)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(Ticket { summary: s, description, links, attempts }))
    }

    /// The backlog: filtered, searched, sorted and paged with a keyset cursor, plus the total that match.
    pub async fn query_tickets(&self, f: &TicketFilter) -> TResult<TicketPage> {
        let mut conn = self.pool.acquire().await?;
        let mut count = QueryBuilder::<Sqlite>::new(format!("SELECT COUNT(*){FROM}"));
        push_filters(&mut count, f);
        let (total,): (i64,) = count.build_query_as().fetch_one(&mut *conn).await?;

        let mut qb = QueryBuilder::<Sqlite>::new(summary_select());
        push_filters(&mut qb, f);
        if let Some(c) = &f.cursor {
            push_cursor(&mut qb, f.sort, c)?;
        }
        qb.push(order_by(f.sort));
        qb.push(" LIMIT ").push_bind(f.limit as i64 + 1);
        let rows = qb.build().fetch_all(&mut *conn).await?;
        let mut items = rows.iter().map(to_summary).collect::<Result<Vec<_>, _>>()?;
        let more = items.len() > f.limit as usize;
        items.truncate(f.limit as usize);
        decorate(&mut conn, &mut items).await?;
        let next_cursor = if more { items.last().map(|t| encode_cursor(f.sort, t)) } else { None };
        Ok(TicketPage { items, next_cursor, total: u32::try_from(total).unwrap_or(u32::MAX) })
    }

    /// Applies an edit, recording `moved` for a stage change and `edited` for the rest. None if there is no such
    /// ticket.
    pub async fn patch_ticket(&self, id: &str, p: &ValidPatch, now: i64) -> TResult<Option<Ticket>> {
        let mut tx = self.pool.begin().await?;
        let Some(cur) = summary(&mut tx, id).await? else { return Ok(None) };
        let (description,): (String,) =
            sqlx::query_as("SELECT description FROM tickets WHERE id = ?").bind(id).fetch_one(&mut *tx).await?;
        let mut fields: Vec<&str> = vec![];
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
        }
        if let Some(l) = p.labels.as_ref().filter(|l| **l != cur.labels) {
            replace_set(&mut tx, "ticket_labels", "label", id, l).await?;
            fields.push("labels");
        }
        if let Some(a) = p.areas.as_ref().filter(|a| **a != cur.areas) {
            replace_set(&mut tx, "ticket_areas", "area", id, a).await?;
            fields.push("areas");
        }
        if !fields.is_empty() {
            touch(&mut tx, id, now).await?;
            insert_event(&mut tx, id, EventKind::Edited, Actor::User, None, Some(json!({"fields": fields})), now)
                .await?;
        }
        if let Some(stage) = p.stage.filter(|s| *s != cur.stage) {
            change_stage(&mut tx, id, cur.stage, stage, now).await?;
        }
        tx.commit().await?;
        Ok(self.get_ticket(id).await?)
    }

    /// Deletes a ticket (its tab, links, labels and events go with it). Refused while it has an active attempt.
    pub async fn delete_ticket(&self, id: &str) -> Result<DeleteTicket, StoreError> {
        let mut tx = self.pool.begin().await?;
        if ticket_head(&mut tx, id).await?.is_none() {
            return Ok(DeleteTicket::NotFound);
        }
        if has_active_attempt(&mut tx, id).await? {
            return Ok(DeleteTicket::Active);
        }
        delete_rows(&mut tx, &[id.to_string()]).await?;
        tx.commit().await?;
        Ok(DeleteTicket::Deleted)
    }

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
        let mut tx = self.pool.begin().await?;
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

    /// Applies one action to every ticket in `ids`, in one transaction: all of them change or none do. Returns how
    /// many changed.
    pub async fn bulk_tickets(&self, ids: &[String], action: &ValidBulk, now: i64) -> TResult<u32> {
        let mut seen = HashSet::new();
        let ids: Vec<String> = ids.iter().filter(|id| seen.insert(id.as_str())).cloned().collect();
        let mut tx = self.pool.begin().await?;
        let mut heads = Vec::with_capacity(ids.len());
        for id in &ids {
            let Some(h) = ticket_head(&mut tx, id).await? else { return Err(TicketError::NotFound("ticket")) };
            heads.push((id.clone(), h));
        }
        let mut count = 0u32;
        match action {
            ValidBulk::Stage(to) => {
                for (id, (_, _, from)) in &heads {
                    if from != to {
                        change_stage(&mut tx, id, *from, *to, now).await?;
                        count += 1;
                    }
                }
            }
            ValidBulk::Priority(pr) => {
                for (id, _) in &heads {
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
                }
                delete_rows(&mut tx, &ids).await?;
                count = ids.len() as u32;
            }
        }
        tx.commit().await?;
        Ok(count)
    }

    /// Records a new active attempt on a ticket (Start, Phase 4b). `Conflict` if the ticket already has one.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_attempt(
        &self,
        id: &str,
        ticket_id: &str,
        worktree_id: Option<&str>,
        branch: &str,
        base_branch: &str,
        base_commit: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO attempts (id, ticket_id, worktree_id, branch, base_branch, base_commit, state, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, 'active', ?)",
        )
        .bind(id)
        .bind(ticket_id)
        .bind(worktree_id)
        .bind(branch)
        .bind(base_branch)
        .bind(base_commit)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every label used in a project, sorted.
    pub async fn project_labels(&self, project_id: &str) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT l.label FROM ticket_labels l JOIN tickets t ON t.id = l.ticket_id \
             WHERE t.project_id = ? ORDER BY l.label",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(l,)| l).collect())
    }

    /// Links `from` to `to` (both in one project). A `blocks` link that would close a cycle is refused with the
    /// cycle's keys. Linking twice (or `relates` in either direction) is a no-op. None if `from` doesn't exist.
    pub async fn add_link(&self, from: &str, to: &str, kind: LinkKind, now: i64) -> TResult<Option<Ticket>> {
        if from == to {
            return Err(TicketError::Invalid("a ticket can't link to itself".into()));
        }
        let mut tx = self.pool.begin().await?;
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
        link_events(&mut tx, EventKind::Linked, kind, (from, &from_key), (to, &to_key), now).await?;
        tx.commit().await?;
        Ok(self.get_ticket(from).await?)
    }

    /// Removes a link (`relates` in either direction). NotFound("link") if there is none; None if `from` doesn't
    /// exist.
    pub async fn remove_link(&self, from: &str, to: &str, kind: LinkKind, now: i64) -> TResult<Option<Ticket>> {
        let mut tx = self.pool.begin().await?;
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
        link_events(&mut tx, EventKind::Unlinked, kind, (from, &from_key), (to, &to_key), now).await?;
        if kind == LinkKind::Blocks && !from_stage.is_closed() {
            let sql = format!("SELECT {BLOCKED_BY} FROM tickets t WHERE t.id = ?");
            let (n,): (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(to).fetch_one(&mut *tx).await?;
            if n == 0 {
                insert_event(&mut tx, to, EventKind::Unblocked, Actor::System, None, Some(json!({"by": from})), now)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(self.get_ticket(from).await?)
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
        let mut tx = self.pool.begin().await?;
        if ticket_head(&mut tx, ticket_id).await?.is_none() {
            return Ok(None);
        }
        let id = insert_event(&mut tx, ticket_id, EventKind::Comment, Actor::User, Some(body), None, now).await?;
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

/// `linked`/`unlinked` on both tickets, each naming the other.
async fn link_events(
    conn: &mut SqliteConnection,
    event: EventKind,
    kind: LinkKind,
    (from, from_key): (&str, &str),
    (to, to_key): (&str, &str),
    now: i64,
) -> Result<(), StoreError> {
    let data = |dir: LinkDirection, other: &str, key: &str| {
        Some(json!({"kind": kind, "direction": dir, "ticketId": other, "key": key}))
    };
    insert_event(conn, from, event, Actor::User, None, data(LinkDirection::Outgoing, to, to_key), now).await?;
    insert_event(conn, to, event, Actor::User, None, data(LinkDirection::Incoming, from, from_key), now).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::project::Project;
    use pdt_core::ticket::TicketQuery;
    use pdt_core::workspace::TabItem;

    fn new(title: &str) -> ValidNewTicket {
        ValidNewTicket {
            title: title.into(),
            description: String::new(),
            stage: TicketStage::Draft,
            priority: Priority::None,
            labels: vec![],
            areas: vec![],
            before: None,
            after: None,
        }
    }

    async fn add(store: &Store, id: &str, t: ValidNewTicket) -> Ticket {
        store.create_ticket(id, "p", &t, 1).await.unwrap()
    }

    fn filter(q: TicketQuery) -> TicketFilter {
        q.validate().unwrap()
    }

    async fn ids(store: &Store, q: TicketQuery) -> Vec<String> {
        store.query_tickets(&filter(q)).await.unwrap().items.into_iter().map(|t| t.id).collect()
    }

    fn in_project() -> TicketQuery {
        TicketQuery { project: Some("p".into()), ..Default::default() }
    }

    async fn events(store: &Store, id: &str) -> Vec<EventKind> {
        store.list_events(id, None, 100, false).await.unwrap().unwrap().items.into_iter().map(|e| e.kind).collect()
    }

    #[tokio::test]
    async fn numbers_count_up_per_project_and_are_never_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let a = add(&store, "a", new("A")).await;
        let b = add(&store, "b", new("B")).await;
        assert_eq!((a.summary.number, b.summary.number), (1, 2));
        assert_eq!(a.summary.key, "PX-1");
        assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::Deleted);
        assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::NotFound);
        assert_eq!(add(&store, "c", new("C")).await.summary.number, 3);

        let q = Project {
            id: "q".into(),
            name: "Other".into(),
            repo_path: "/q".into(),
            base_branch: "main".into(),
            created_at: 2,
            key: "OTH".into(),
            github_repo: None,
        };
        store.insert_project(&q).await.unwrap();
        let t = store.create_ticket("q1", "q", &new("Q"), 1).await.unwrap();
        assert_eq!(t.summary.key, "OTH-1");
        assert!(matches!(store.create_ticket("x", "nope", &new("X"), 1).await, Err(TicketError::NotFound("project"))));

        // Removing the project removes its tickets.
        assert!(store.delete_project("q").await.unwrap());
        assert!(store.get_ticket("q1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn new_tickets_go_to_the_top_of_their_stage_or_where_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "r1", ValidNewTicket { stage: TicketStage::Ready, ..new("R1") }).await;
        add(&store, "d1", new("D1")).await;
        add(&store, "r2", ValidNewTicket { stage: TicketStage::Ready, ..new("R2") }).await;
        add(&store, "d2", new("D2")).await;
        // A stage's first ticket goes to the top of the whole backlog.
        assert_eq!(ids(&store, in_project()).await, ["d2", "d1", "r2", "r1"]);
        add(&store, "x", ValidNewTicket { before: Some("d1".into()), ..new("X") }).await;
        add(&store, "y", ValidNewTicket { after: Some("r2".into()), ..new("Y") }).await;
        assert_eq!(ids(&store, in_project()).await, ["d2", "x", "d1", "r2", "y", "r1"]);
        let bad = ValidNewTicket { before: Some("d2".into()), after: Some("d1".into()), ..new("Z") };
        assert!(matches!(store.create_ticket("z", "p", &bad, 1).await, Err(TicketError::Invalid(_))));
        let bad = ValidNewTicket { before: Some("nope".into()), ..new("Z") };
        assert!(matches!(store.create_ticket("z", "p", &bad, 1).await, Err(TicketError::Invalid(_))));
        assert!(store.get_ticket("z").await.unwrap().is_none(), "a refused create leaves nothing");
    }

    #[tokio::test]
    async fn moving_writes_one_rank_and_rebalances_long_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        for id in ["c", "b", "a"] {
            add(&store, id, new(id)).await;
        }
        assert_eq!(ids(&store, in_project()).await, ["a", "b", "c"]);
        store.move_ticket("a", None, Some("c"), None).await.unwrap().unwrap();
        assert_eq!(ids(&store, in_project()).await, ["b", "c", "a"]);
        store.move_ticket("a", Some("c"), None, None).await.unwrap().unwrap();
        assert_eq!(ids(&store, in_project()).await, ["b", "a", "c"]);
        store.move_ticket("c", None, None, Some(RankEdge::Top)).await.unwrap().unwrap();
        assert_eq!(ids(&store, in_project()).await, ["c", "b", "a"]);
        store.move_ticket("c", None, None, Some(RankEdge::Bottom)).await.unwrap().unwrap();
        assert_eq!(ids(&store, in_project()).await, ["b", "a", "c"]);
        assert!(store.move_ticket("nope", Some("a"), None, None).await.unwrap().is_none());
        assert!(matches!(store.move_ticket("a", Some("a"), None, None).await, Err(TicketError::Invalid(_))));
        assert!(matches!(store.move_ticket("a", None, None, None).await, Err(TicketError::Invalid(_))));

        // Swap the last two over and over: the same gap is split each time, until a rebalance resets the keys.
        for i in 0..400 {
            let (x, y) = if i % 2 == 0 { ("c", "a") } else { ("a", "c") };
            store.move_ticket(x, Some(y), None, None).await.unwrap();
        }
        let page = store.query_tickets(&filter(in_project())).await.unwrap();
        assert_eq!(page.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "a", "c"]);
        assert!(page.items.iter().all(|t| t.rank.len() <= rank::REBALANCE_LEN), "{:?}", page.items);
    }

    #[tokio::test]
    async fn search_matches_word_prefixes_and_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "a", ValidNewTicket { description: "The login form crashes".into(), ..new("Fix crash") }).await;
        add(&store, "b", new("Café menu layout")).await;
        add(&store, "c", new("Logout button")).await;
        let q = |s: &str| TicketQuery { q: Some(s.into()), ..in_project() };
        assert_eq!(ids(&store, q("log")).await, ["c", "a"]);
        assert_eq!(ids(&store, q("login cra")).await, ["a"]);
        assert_eq!(ids(&store, q("cafe")).await, ["b"]);
        assert_eq!(ids(&store, q("PX-2")).await, ["b"]);
        assert_eq!(ids(&store, q("px-3")).await, ["c"]);
        assert_eq!(ids(&store, q("1")).await, ["a"]);
        assert_eq!(ids(&store, q("OTHER-1")).await, Vec::<String>::new());
        assert_eq!(ids(&store, q("\"quoted\" (stuff) *")).await, Vec::<String>::new());

        // Edits re-index; deletes un-index.
        let patch =
            ValidPatch { title: Some("Sign-in crash".into()), description: Some(String::new()), ..Default::default() };
        store.patch_ticket("a", &patch, 2000).await.unwrap();
        assert_eq!(ids(&store, q("login")).await, Vec::<String>::new());
        assert_eq!(ids(&store, q("sign")).await, ["a"]);
        store.delete_ticket("c").await.unwrap();
        assert_eq!(ids(&store, q("logout")).await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn filters_combine() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        add(&store, "a", ValidNewTicket { labels: l(&["bug", "ui"]), priority: Priority::High, ..new("A") }).await;
        add(&store, "b", ValidNewTicket { labels: l(&["bug"]), stage: TicketStage::Ready, ..new("B") }).await;
        add(&store, "c", ValidNewTicket { labels: l(&["ui"]), stage: TicketStage::Done, ..new("C") }).await;
        let page = store.query_tickets(&filter(in_project())).await.unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.items.iter().find(|t| t.id == "a").unwrap().labels, ["bug", "ui"]);
        assert!(page.items.iter().find(|t| t.id == "c").unwrap().closed_at.is_some());
        let mut got = ids(&store, TicketQuery { label: Some("bug".into()), ..in_project() }).await;
        got.sort();
        assert_eq!(got, ["a", "b"]);
        assert_eq!(ids(&store, TicketQuery { label: Some("bug,ui".into()), ..in_project() }).await, ["a"]);
        assert_eq!(ids(&store, TicketQuery { priority: Some("high".into()), ..in_project() }).await, ["a"]);
        let open = ids(&store, TicketQuery { stage: Some("draft,ready".into()), ..in_project() }).await;
        assert_eq!(open.len(), 2);
        assert_eq!(store.project_labels("p").await.unwrap(), ["bug", "ui"]);

        // b blocks a: a is blocked until b closes.
        store.add_link("b", "a", LinkKind::Blocks, 1).await.unwrap();
        assert_eq!(ids(&store, TicketQuery { blocked: Some(true), ..in_project() }).await, ["a"]);
        assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().blocked_by, 1);
        store
            .patch_ticket("b", &ValidPatch { stage: Some(TicketStage::WontDo), ..Default::default() }, 5)
            .await
            .unwrap();
        assert_eq!(ids(&store, TicketQuery { blocked: Some(true), ..in_project() }).await, Vec::<String>::new());
        assert_eq!(ids(&store, TicketQuery { blocked: Some(false), ..in_project() }).await.len(), 3);
        assert_eq!(events(&store, "a").await.last(), Some(&EventKind::Unblocked));
    }

    #[tokio::test]
    async fn a_thousand_tickets_page_and_sort_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let stages = [TicketStage::Draft, TicketStage::Ready, TicketStage::InProgress, TicketStage::Done];
        let mut tx = store.pool.begin().await.unwrap();
        let keys = rank::spread(1000);
        for i in 0..1000usize {
            // Inserted directly (one transaction) to keep the test quick; the API path is covered elsewhere.
            sqlx::query(
                "INSERT INTO tickets (id, seq, project_id, number, title, stage, priority, rank, created_at, updated_at) \
                 VALUES (?, ?, 'p', ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("t{i:04}"))
            .bind(i as i64 + 1)
            .bind(i as i64 + 1)
            .bind(format!("Ticket {i} {}", if i % 10 == 0 { "searchable" } else { "plain" }))
            .bind(stages[i % 4].as_str())
            .bind(Priority::ALL[i % 5].as_str())
            .bind(&keys[(i * 7919) % 1000])
            .bind((i % 50) as i64)
            .bind(((i * 31) % 997) as i64)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let mut all = store.query_tickets(&filter(TicketQuery { limit: Some(500), ..in_project() })).await.unwrap();
        assert_eq!(all.total, 1000);
        let rest = TicketQuery { limit: Some(500), cursor: all.next_cursor.clone(), ..in_project() };
        all.items.extend(store.query_tickets(&filter(rest)).await.unwrap().items);
        assert_eq!(all.items.len(), 1000);

        for sort in TicketSort::ALL {
            let mut expected = all.items.clone();
            match sort {
                TicketSort::Rank => expected.sort_by(|a, b| (&a.rank, &a.id).cmp(&(&b.rank, &b.id))),
                TicketSort::Priority => expected.sort_by(|a, b| {
                    (std::cmp::Reverse(a.priority.weight()), &a.rank, &a.id).cmp(&(
                        std::cmp::Reverse(b.priority.weight()),
                        &b.rank,
                        &b.id,
                    ))
                }),
                TicketSort::Updated => expected.sort_by(|a, b| (b.updated_at, &b.id).cmp(&(a.updated_at, &a.id))),
                TicketSort::Created => expected.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id))),
            }
            for stage_filter in [None, Some("ready,done")] {
                let want: Vec<&str> = expected
                    .iter()
                    .filter(|t| stage_filter.is_none() || matches!(t.stage, TicketStage::Ready | TicketStage::Done))
                    .map(|t| t.id.as_str())
                    .collect();
                let mut got: Vec<String> = vec![];
                let mut cursor = None;
                loop {
                    let q = TicketQuery {
                        sort: Some(*sort),
                        limit: Some(37),
                        cursor,
                        stage: stage_filter.map(Into::into),
                        ..in_project()
                    };
                    let page = store.query_tickets(&filter(q)).await.unwrap();
                    assert_eq!(page.total as usize, want.len());
                    got.extend(page.items.into_iter().map(|t| t.id));
                    match page.next_cursor {
                        Some(c) => cursor = Some(c),
                        None => break,
                    }
                }
                assert_eq!(got, want, "{sort:?} {stage_filter:?}");
            }
        }
        let page = store
            .query_tickets(&filter(TicketQuery { q: Some("search".into()), limit: Some(30), ..in_project() }))
            .await
            .unwrap();
        assert_eq!((page.total, page.items.len(), page.next_cursor.is_some()), (100, 30, true));
        let bad = TicketQuery { sort: Some(TicketSort::Updated), cursor: Some("r:V:x".into()), ..in_project() };
        assert!(matches!(store.query_tickets(&filter(bad)).await, Err(TicketError::Invalid(_))));
    }

    #[tokio::test]
    async fn edits_record_events_and_closing_sets_closed_at() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "a", new("A")).await;
        let p = ValidPatch {
            title: Some("A2".into()),
            labels: Some(vec!["x".into()]),
            stage: Some(TicketStage::Done),
            ..Default::default()
        };
        let t = store.patch_ticket("a", &p, 2000).await.unwrap().unwrap();
        assert_eq!(
            (t.summary.title.as_str(), t.summary.stage, t.summary.closed_at),
            ("A2", TicketStage::Done, Some(2000))
        );
        assert_eq!(t.summary.updated_at, 2000);
        let p = ValidPatch { stage: Some(TicketStage::WontDo), ..Default::default() };
        let t = store.patch_ticket("a", &p, 3000).await.unwrap().unwrap();
        assert_eq!(t.summary.closed_at, Some(2000), "still closed since 2000");
        let p = ValidPatch { stage: Some(TicketStage::Ready), title: Some("A2".into()), ..Default::default() };
        let t = store.patch_ticket("a", &p, 4000).await.unwrap().unwrap();
        assert_eq!(t.summary.closed_at, None);
        assert_eq!(
            events(&store, "a").await,
            [EventKind::Created, EventKind::Edited, EventKind::Moved, EventKind::Moved, EventKind::Moved]
        );
        let page = store.list_events("a", None, 100, false).await.unwrap().unwrap();
        assert_eq!(page.items[1].data, Some(json!({"fields": ["title", "labels"]})));
        assert_eq!(page.items[2].data, Some(json!({"from": "draft", "to": "done"})));
        assert!(store.patch_ticket("nope", &ValidPatch::default(), 1).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn bulk_actions_are_all_or_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        for id in ["c", "b", "a"] {
            add(&store, id, new(id)).await;
        }
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let r = store.bulk_tickets(&v(&["a", "nope"]), &ValidBulk::Stage(TicketStage::Ready), 5).await;
        assert!(matches!(r, Err(TicketError::NotFound("ticket"))));
        assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().stage, TicketStage::Draft);

        assert_eq!(
            store.bulk_tickets(&v(&["a", "b", "a"]), &ValidBulk::Stage(TicketStage::Ready), 5).await.unwrap(),
            2
        );
        assert_eq!(store.bulk_tickets(&v(&["a", "c"]), &ValidBulk::Stage(TicketStage::Ready), 5).await.unwrap(), 1);
        assert_eq!(store.bulk_tickets(&v(&["a"]), &ValidBulk::Priority(Priority::Urgent), 5).await.unwrap(), 1);
        assert_eq!(store.bulk_tickets(&v(&["a", "b"]), &ValidBulk::AddLabel("ui".into()), 5).await.unwrap(), 2);
        assert_eq!(store.bulk_tickets(&v(&["a", "b"]), &ValidBulk::RemoveLabel("ui".into()), 5).await.unwrap(), 2);
        assert_eq!(store.bulk_tickets(&v(&["c"]), &ValidBulk::RemoveLabel("ui".into()), 5).await.unwrap(), 0);

        // One ticket over the label limit undoes the whole action.
        let full: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
        store.patch_ticket("b", &ValidPatch { labels: Some(full), ..Default::default() }, 6).await.unwrap();
        let r = store.bulk_tickets(&v(&["a", "b"]), &ValidBulk::AddLabel("new".into()), 7).await;
        assert!(matches!(r, Err(TicketError::Invalid(_))));
        assert!(store.get_ticket_summary("a").await.unwrap().unwrap().labels.is_empty());

        assert_eq!(ids(&store, in_project()).await, ["a", "b", "c"]);
        store.bulk_tickets(&v(&["c", "b"]), &ValidBulk::Top, 8).await.unwrap();
        assert_eq!(ids(&store, in_project()).await, ["b", "c", "a"]);

        // An active attempt blocks a bulk delete of any of them.
        sqlx::query(
            "INSERT INTO attempts (id, ticket_id, branch, base_branch, base_commit, state, created_at) \
             VALUES ('at', 'a', 'pdt/x', 'main', 'abc', 'active', 1)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        let r = store.bulk_tickets(&v(&["b", "a"]), &ValidBulk::Delete, 9).await;
        assert!(matches!(r, Err(TicketError::Active(k)) if k == "PX-3"));
        assert_eq!(store.delete_ticket("a").await.unwrap(), DeleteTicket::Active);
        assert_eq!(ids(&store, in_project()).await.len(), 3);
        assert_eq!(store.bulk_tickets(&v(&["b", "c"]), &ValidBulk::Delete, 9).await.unwrap(), 2);
        assert_eq!(ids(&store, in_project()).await, ["a"]);
        let summary = store.get_ticket_summary("a").await.unwrap().unwrap();
        assert_eq!(summary.attempt.map(|a| a.state), Some(AttemptState::Active));

        // One active attempt per ticket.
        let dup = sqlx::query(
            "INSERT INTO attempts (id, ticket_id, branch, base_branch, base_commit, state, created_at) \
             VALUES ('at2', 'a', 'pdt/y', 'main', 'abc', 'active', 2)",
        )
        .execute(&store.pool)
        .await;
        assert!(dup.is_err());
    }

    #[tokio::test]
    async fn links_refuse_cycles_and_show_on_both_tickets() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        for id in ["a", "b", "c"] {
            add(&store, id, new(id)).await;
        }
        store.add_link("a", "b", LinkKind::Blocks, 1).await.unwrap();
        store.add_link("b", "c", LinkKind::Blocks, 1).await.unwrap();
        match store.add_link("c", "a", LinkKind::Blocks, 1).await {
            Err(TicketError::Cycle(keys)) => assert_eq!(keys, ["PX-3", "PX-1", "PX-2", "PX-3"]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(store.add_link("a", "a", LinkKind::Relates, 1).await, Err(TicketError::Invalid(_))));
        assert!(matches!(store.add_link("a", "nope", LinkKind::Relates, 1).await, Err(TicketError::Invalid(_))));
        assert!(store.add_link("nope", "a", LinkKind::Relates, 1).await.unwrap().is_none());

        store.add_link("c", "a", LinkKind::Relates, 1).await.unwrap();
        let t = store.add_link("a", "c", LinkKind::Relates, 1).await.unwrap().unwrap();
        let summary: Vec<(LinkKind, LinkDirection, &str)> =
            t.links.iter().map(|l| (l.kind, l.direction, l.key.as_str())).collect();
        assert_eq!(
            summary,
            [(LinkKind::Blocks, LinkDirection::Outgoing, "PX-2"), (LinkKind::Relates, LinkDirection::Incoming, "PX-3")]
        );
        assert_eq!(store.get_ticket("b").await.unwrap().unwrap().links.len(), 2);

        // relates comes off from either side; blocks only as it was made.
        store.remove_link("a", "c", LinkKind::Relates, 2).await.unwrap();
        assert!(matches!(store.remove_link("b", "a", LinkKind::Blocks, 2).await, Err(TicketError::NotFound("link"))));
        store.remove_link("a", "b", LinkKind::Blocks, 2).await.unwrap();
        assert_eq!(
            events(&store, "b").await,
            [EventKind::Created, EventKind::Linked, EventKind::Linked, EventKind::Unlinked, EventKind::Unblocked]
        );
        store.delete_ticket("c").await.unwrap();
        assert!(store.get_ticket("b").await.unwrap().unwrap().links.is_empty());
    }

    #[tokio::test]
    async fn comments_and_timeline_paging() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "a", new("A")).await;
        for i in 0..5 {
            let e = store.add_comment("a", &format!("c{i}"), 2000 + i).await.unwrap().unwrap();
            assert_eq!((e.kind, e.body.as_deref()), (EventKind::Comment, Some(format!("c{i}").as_str())));
        }
        assert!(store.add_comment("nope", "x", 1).await.unwrap().is_none());
        assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().updated_at, 2004);
        let mut seen = vec![];
        let mut cursor = None;
        loop {
            let page = store.list_events("a", cursor.as_deref(), 2, true).await.unwrap().unwrap();
            seen.extend(page.items.iter().map(|e| e.body.clone().unwrap_or_default()));
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(seen, ["c4", "c3", "c2", "c1", "c0", ""]);
        assert!(store.list_events("nope", None, 2, false).await.unwrap().is_none());
        assert!(matches!(store.list_events("a", Some("junk"), 2, false).await, Err(TicketError::Invalid(_))));
    }

    #[tokio::test]
    async fn tickets_open_in_tabs_and_take_their_tab_with_them() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let home = store.project_workspace_id("p").await.unwrap().unwrap();
        for id in ["a", "b"] {
            add(&store, id, new(id)).await;
            store.open_tab(&format!("t{id}"), &home, &TabItem::Ticket(id.into()), None, 1).await.unwrap().unwrap();
        }
        let tab = store.tab_for(&TabItem::Ticket("b".into())).await.unwrap().unwrap();
        assert_eq!((tab.id.as_str(), tab.ticket_id.as_deref(), tab.position), ("tb", Some("b"), 1));
        // One tab per ticket: opening again moves it.
        store.open_tab("ignored", &home, &TabItem::Ticket("b".into()), Some(0), 1).await.unwrap();
        assert_eq!(store.get_workspace(&home).await.unwrap().unwrap().tabs[0].id, "tb");

        // A tab shows exactly one thing.
        let bad = sqlx::query(
            "INSERT INTO workspace_tabs (id, workspace_id, session_id, run_id, ticket_id, position, created_at) \
             VALUES ('x', ?, NULL, NULL, NULL, 9, 1)",
        )
        .bind(&home)
        .execute(&store.pool)
        .await;
        assert!(bad.is_err());

        // A free-form workspace's ticket tabs go home when it is deleted.
        let free = pdt_core::workspace::Workspace {
            id: "f".into(),
            name: "F".into(),
            project_id: None,
            position: 0,
            created_at: 1,
            tabs: vec![],
        };
        store.insert_workspace(&free).await.unwrap();
        store.move_tab("ta", Some("f"), None).await.unwrap().unwrap();
        store.delete_workspace("f").await.unwrap();
        assert_eq!(store.get_workspace(&home).await.unwrap().unwrap().tabs.len(), 2);

        assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::Deleted);
        let tabs = store.get_workspace(&home).await.unwrap().unwrap().tabs;
        assert_eq!(tabs.iter().map(|t| (t.id.as_str(), t.position)).collect::<Vec<_>>(), [("ta", 0)]);
    }
}
