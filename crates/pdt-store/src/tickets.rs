//! Tickets (tickets spec §3, §4): numbering, ranks, the backlog query (filters, full-text search, keyset paging),
//! edits with their timeline events, bulk actions, links and comments.

use crate::{Store, StoreError};
use pdt_core::conversation::SessionMode;
use pdt_core::links::would_cycle;
use pdt_core::parts::{ParentFacts, TicketParent, ValidParts, open_parts, valid_parent};
use pdt_core::rank;
use pdt_core::session::{AgentKind, SessionStatus};
use pdt_core::start::attempt_branch;
use pdt_core::ticket::{
    Actor, AttemptSession, AttemptState, AttemptSummary, EventKind, EventPage, LinkDirection, LinkKind, LinkedTicket,
    MAX_LABELS, OverlapLevel, ParentFilter, Priority, RankEdge, Ticket, TicketEvent, TicketFilter, TicketPage,
    TicketSort, TicketStage, TicketSummary, ValidBulk, ValidNewTicket, ValidPatch, format_key, parse_key,
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
    /// The ticket can't be that parent's part (PDT-38).
    #[error("{0}")]
    InvalidParent(pdt_core::parts::ParentRule),
    /// A parent (by key) can't close while these parts are open.
    #[error("{key} still has open parts: {}", open.join(", "))]
    PartsOpen { key: String, open: Vec<String> },
    /// The ticket (by key) has parts, so it can't be deleted.
    #[error("{0} has parts; delete or detach them first")]
    HasParts(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<pdt_core::parts::ParentRule> for TicketError {
    fn from(e: pdt_core::parts::ParentRule) -> Self {
        TicketError::InvalidParent(e)
    }
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
    /// It has parts.
    HasParts,
}

/// What `start_attempt` changed, so `undo_start` can take it back.
#[derive(Debug, Clone, PartialEq)]
pub struct StartedAttempt {
    pub attempt: AttemptSummary,
    pub ticket_id: String,
    prev_stage: TicketStage,
    prev_updated_at: i64,
    event_ids: Vec<String>,
}

/// An attempt with its ticket, project and worktree: what Start's rollback, abandon and merge detection need.
#[derive(Debug, Clone, PartialEq)]
pub struct AttemptInfo {
    pub attempt: AttemptSummary,
    pub ticket_id: String,
    /// `PDT-42`.
    pub ticket_key: String,
    pub project_id: String,
    pub repo_path: String,
    pub base_commit: String,
    /// Merge detection has seen the branch hold commits that are not in its base (sticky).
    pub own_commits: bool,
    /// None when the worktree record is gone.
    pub worktree_path: Option<String>,
}

const ATTEMPT_SELECT: &str = "SELECT a.id, a.ticket_id, a.branch, a.base_branch, a.worktree_id, a.state, a.created_at, \
                              a.ended_at, a.last_head, a.workspace_id FROM attempts a";
const ATTEMPT_INFO_SELECT: &str = "SELECT a.id, a.ticket_id, a.branch, a.base_branch, a.worktree_id, a.state, \
                                   a.created_at, a.ended_at, a.last_head, a.workspace_id, a.base_commit, a.own_commits, \
                                   t.project_id, COALESCE(p.key, '') AS pkey, t.number, p.repo_path, \
                                   w.path AS worktree_path \
                                   FROM attempts a JOIN tickets t ON t.id = a.ticket_id \
                                   JOIN projects p ON p.id = t.project_id LEFT JOIN worktrees w ON w.id = a.worktree_id";

fn to_attempt_info(r: &SqliteRow) -> Result<AttemptInfo, StoreError> {
    let pkey: String = r.try_get("pkey")?;
    Ok(AttemptInfo {
        attempt: to_attempt(r)?,
        ticket_id: r.try_get("ticket_id")?,
        ticket_key: format_key(&pkey, u32::try_from(r.try_get::<i64, _>("number")?).unwrap_or(0)),
        project_id: r.try_get("project_id")?,
        repo_path: r.try_get("repo_path")?,
        base_commit: r.try_get("base_commit")?,
        own_commits: r.try_get::<i64, _>("own_commits")? != 0,
        worktree_path: r.try_get("worktree_path")?,
    })
}

/// An open ticket for overlap detection (see `Store::overlap_rows`).
#[derive(Debug, Clone, PartialEq)]
pub struct OverlapRow {
    pub ticket_id: String,
    pub project_id: String,
    pub key: String,
    pub number: u32,
    pub title: String,
    pub stage: TicketStage,
    /// Its active attempt, if any.
    pub attempt_id: Option<String>,
    /// The active attempt's worktree.
    pub worktree_id: Option<String>,
    /// Sorted.
    pub areas: Vec<String>,
}

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

/// Priority as a number (higher is more urgent), for sorting.
const PRIORITY_WEIGHT: &str = "(CASE t.priority WHEN 'urgent' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 \
                               WHEN 'low' THEN 1 ELSE 0 END)";
/// How many open tickets block `t`.
const BLOCKED_BY: &str = "(SELECT COUNT(*) FROM ticket_links l JOIN tickets b ON b.id = l.from_id \
                          WHERE l.to_id = t.id AND l.kind = 'blocks' AND b.stage NOT IN ('done', 'wont_do'))";
const FROM: &str =
    " FROM tickets t JOIN projects p ON p.id = t.project_id LEFT JOIN tickets par ON par.id = t.parent_id";

fn summary_select() -> String {
    format!(
        "SELECT t.id, t.project_id, COALESCE(p.key, '') AS pkey, t.number, t.title, t.stage, t.priority, t.rank, \
         t.waiting, t.created_at, t.updated_at, t.closed_at, {BLOCKED_BY} AS blocked_by, {PRIORITY_WEIGHT} AS pw, \
         t.parent_id, par.number AS parent_number, par.title AS parent_title{FROM}"
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
        clear_to_start: false,
        blocked_by: u32::try_from(r.try_get::<i64, _>("blocked_by")?).unwrap_or(0),
        attempt: None,
        overlap: OverlapLevel::None,
        overlap_with: vec![],
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        closed_at: r.try_get("closed_at")?,
        parent: match r.try_get::<Option<String>, _>("parent_id")? {
            Some(id) => Some(TicketParent {
                id,
                key: format_key(&pkey, u32::try_from(r.try_get::<i64, _>("parent_number")?).unwrap_or(0)),
                title: r.try_get("parent_title")?,
            }),
            None => None,
        },
        parts: None,
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
        last_head: r.try_get("last_head")?,
        workspace_id: r.try_get("workspace_id")?,
        pr: None,
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
            "SELECT id, ticket_id, branch, base_branch, worktree_id, state, created_at, ended_at, last_head, workspace_id FROM attempts \
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
    if let Some(ids) = &f.only_ids {
        if ids.is_empty() {
            qb.push(" AND 0");
        } else {
            qb.push(" AND t.id IN ");
            push_in(qb, ids);
        }
    }
    if !f.except_ids.is_empty() {
        qb.push(" AND t.id NOT IN ");
        push_in(qb, &f.except_ids);
    }
    match &f.parent {
        Some(ParentFilter::Of(id)) => {
            qb.push(" AND t.parent_id = ").push_bind(id.clone());
        }
        Some(ParentFilter::None) => {
            qb.push(" AND t.parent_id IS NULL");
        }
        None => {}
    }
    match f.has_parts {
        Some(true) => {
            qb.push(" AND EXISTS (SELECT 1 FROM tickets c WHERE c.parent_id = t.id)");
        }
        Some(false) => {
            qb.push(" AND NOT EXISTS (SELECT 1 FROM tickets c WHERE c.parent_id = t.id)");
        }
        None => {}
    }
    if let Some(after) = f.closed_after {
        qb.push(" AND (t.closed_at IS NULL OR t.closed_at >= ").push_bind(after).push(")");
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
    // A closed ticket no longer waits to start.
    sqlx::query(
        "UPDATE tickets SET stage = ?, updated_at = ?, \
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

/// The facts `valid_parent` needs for making `child` (None: a new ticket) a part of `parent`.
async fn parent_facts(
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
async fn parted_events(
    conn: &mut SqliteConnection,
    (parent, parent_key): (&str, &str),
    (part, part_key): (&str, &str),
    removed: bool,
    now: i64,
) -> Result<(), StoreError> {
    let data =
        json!({"parentId": parent, "parentKey": parent_key, "partId": part, "partKey": part_key, "removed": removed});
    insert_event(conn, parent, EventKind::Parted, Actor::User, None, Some(data.clone()), now).await?;
    insert_event(conn, part, EventKind::Parted, Actor::User, None, Some(data), now).await?;
    Ok(())
}

/// Inserts a ticket with the project's next number (see `Store::create_ticket`). A part (`t.parent`) is checked
/// against the parent rules, goes after the parent's last part (or right after the parent) unless `before`/`after`
/// say otherwise, and with `blocked_by_previous` is blocked by that last part.
async fn insert_ticket(
    conn: &mut SqliteConnection,
    id: &str,
    project_id: &str,
    t: &ValidNewTicket,
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
    sqlx::query(
        "INSERT INTO tickets (id, seq, project_id, number, title, description, stage, priority, rank, created_at, \
         updated_at, closed_at, parent_id) \
         VALUES (?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM tickets), ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    insert_event(conn, id, EventKind::Created, Actor::User, None, Some(json!({"stage": t.stage})), now).await?;
    if let Some(parent) = &t.parent {
        let (pkey,): (String,) = sqlx::query_as("SELECT COALESCE(key, '') FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_one(&mut *conn)
            .await?;
        let own_key = format_key(&pkey, u32::try_from(number).unwrap_or(0));
        parted_events(conn, (parent, &parent_key), (id, &own_key), false, now).await?;
        if let (true, Some(prev)) = (t.blocked_by_previous, previous) {
            sqlx::query("INSERT INTO ticket_links (from_id, to_id, kind, created_at) VALUES (?, ?, 'blocks', ?)")
                .bind(&prev)
                .bind(id)
                .bind(now)
                .execute(&mut *conn)
                .await?;
            let prev_key = ticket_head(conn, &prev).await?.map(|h| h.1).unwrap_or_default();
            link_events(conn, EventKind::Linked, LinkKind::Blocks, (&prev, &prev_key), (id, &own_key), now).await?;
        }
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
    /// Adds a ticket with the project's next number, at the top of its stage (or where `before`/`after` say), with a
    /// `created` event. NotFound("project") if there is no such project.
    pub async fn create_ticket(&self, id: &str, project_id: &str, t: &ValidNewTicket, now: i64) -> TResult<Ticket> {
        let mut tx = self.pool.begin().await?;
        insert_ticket(&mut tx, id, project_id, t, now).await?;
        tx.commit().await?;
        self.get_ticket(id).await?.ok_or(TicketError::NotFound("ticket"))
    }

    /// Makes several parts of `parent`, in order after its existing parts, in one transaction (PDT-38). With
    /// `blocked_by_previous`, each is blocked by the part before it.
    pub async fn create_parts(&self, parent: &str, v: &ValidParts, now: i64) -> TResult<Vec<TicketSummary>> {
        let mut tx = self.pool.begin().await?;
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
            };
            insert_ticket(&mut tx, &id, &project_id, &t, now).await?;
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
        let mut tx = self.pool.begin().await?;
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
            "SELECT id, ticket_id, branch, base_branch, worktree_id, state, created_at, ended_at, last_head, workspace_id FROM attempts \
             WHERE ticket_id = ? ORDER BY created_at DESC, id DESC",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(to_attempt)
        .collect::<Result<Vec<_>, _>>()?;
        let sessions = sqlx::query(
            "SELECT s.id, s.title, s.status, s.agent, s.attempt_id, s.mode FROM sessions s \
             JOIN attempts a ON a.id = s.attempt_id WHERE a.ticket_id = ? ORDER BY s.created_at DESC, s.id DESC",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|r| {
            let status: String = r.try_get("status")?;
            let agent: Option<String> = r.try_get("agent")?;
            let mode: String = r.try_get("mode")?;
            Ok(AttemptSession {
                id: r.try_get("id")?,
                title: r.try_get("title")?,
                status: SessionStatus::parse(&status).ok_or_else(|| corrupt("session status", &status))?,
                agent: agent.map(|a| AgentKind::parse(&a).ok_or_else(|| corrupt("session agent", &a))).transpose()?,
                attempt_id: r.try_get("attempt_id")?,
                mode: SessionMode::parse(&mode).ok_or_else(|| corrupt("session mode", &mode))?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
        let pkey = s.key.rsplit_once('-').map(|(k, _)| k).unwrap_or("");
        let default_branch = attempt_branch(pkey, s.number, &s.title);
        let sql = format!("{} WHERE t.parent_id = ? ORDER BY t.rank, t.id", summary_select());
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_all(&mut *conn).await?;
        let mut part_list = rows.iter().map(to_summary).collect::<Result<Vec<_>, _>>()?;
        decorate(&mut conn, &mut part_list).await?;
        Ok(Some(Ticket {
            summary: s,
            description,
            links,
            attempts,
            default_branch,
            sessions,
            overlaps: vec![],
            part_list,
        }))
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
        if let Some(parent) = &p.parent {
            let current = cur.parent.as_ref().map(|x| x.id.as_str());
            if current != parent.as_deref() {
                if let Some(old) = &cur.parent {
                    parted_events(&mut tx, (&old.id, &old.key), (id, &cur.key), true, now).await?;
                }
                if let Some(new) = parent {
                    let (facts, key, _) = parent_facts(&mut tx, Some((id, &cur.project_id)), new).await?;
                    valid_parent(&facts)?;
                    parted_events(&mut tx, (new, &key), (id, &cur.key), false, now).await?;
                }
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
            change_stage(&mut tx, id, cur.stage, stage, now).await?;
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
                insert_event(
                    &mut tx,
                    id,
                    EventKind::Waiting,
                    Actor::User,
                    None,
                    Some(json!({"waiting": waiting})),
                    now,
                )
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
        let mut tx = self.pool.begin().await?;
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

    /// Starts an attempt on a ticket in one transaction: the attempt row (active), the ticket moved to In progress
    /// (with a `moved` event when that changes it) and a `started` event whose data is `{attemptId, branch,
    /// baseBranch}` plus `extra`'s fields. `Active` if the ticket already has an active attempt; NotFound("ticket").
    #[allow(clippy::too_many_arguments)]
    pub async fn start_attempt(
        &self,
        id: &str,
        ticket_id: &str,
        worktree_id: Option<&str>,
        branch: &str,
        base_branch: &str,
        base_commit: &str,
        extra: serde_json::Value,
        now: i64,
    ) -> TResult<StartedAttempt> {
        let mut tx = self.pool.begin().await?;
        let Some((_, key, stage)) = ticket_head(&mut tx, ticket_id).await? else {
            return Err(TicketError::NotFound("ticket"));
        };
        if has_active_attempt(&mut tx, ticket_id).await? {
            return Err(TicketError::Active(key));
        }
        let (prev_updated_at,): (i64,) =
            sqlx::query_as("SELECT updated_at FROM tickets WHERE id = ?").bind(ticket_id).fetch_one(&mut *tx).await?;
        // Events made from here on (in this transaction) are this start's, for `undo_start`.
        let marker = uuid::Uuid::now_v7().to_string();
        let inserted = sqlx::query(
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
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => {}
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(TicketError::Active(key)),
            Err(e) => return Err(e.into()),
        }
        if stage != TicketStage::InProgress {
            change_stage(&mut tx, ticket_id, stage, TicketStage::InProgress, now).await?;
        }
        // Started: no longer waiting (undo_start does not restore it; Wait can be chosen again).
        sqlx::query("UPDATE tickets SET waiting = 0 WHERE id = ?").bind(ticket_id).execute(&mut *tx).await?;
        let mut data = json!({"attemptId": id, "branch": branch, "baseBranch": base_branch});
        if let (Some(d), serde_json::Value::Object(extra)) = (data.as_object_mut(), extra) {
            d.extend(extra);
        }
        insert_event(&mut tx, ticket_id, EventKind::Started, Actor::User, None, Some(data), now).await?;
        touch(&mut tx, ticket_id, now).await?;
        let event_ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM ticket_events WHERE ticket_id = ? AND id > ?")
            .bind(ticket_id)
            .bind(&marker)
            .fetch_all(&mut *tx)
            .await?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{ATTEMPT_SELECT} WHERE a.id = ?")))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let attempt = to_attempt(&row)?;
        tx.commit().await?;
        Ok(StartedAttempt {
            attempt,
            ticket_id: ticket_id.to_string(),
            prev_stage: stage,
            prev_updated_at,
            event_ids: event_ids.into_iter().map(|(e,)| e).collect(),
        })
    }

    /// Takes back a `start_attempt` whose session could not be made: the attempt, its events and the stage change go;
    /// the ticket is as it was (the worktree is the caller's).
    pub async fn undo_start(&self, started: &StartedAttempt) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM attempts WHERE id = ?").bind(&started.attempt.id).execute(&mut *tx).await?;
        for e in &started.event_ids {
            sqlx::query("DELETE FROM ticket_events WHERE id = ?").bind(e).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE tickets SET stage = ?, updated_at = ? WHERE id = ?")
            .bind(started.prev_stage.as_str())
            .bind(started.prev_updated_at)
            .bind(&started.ticket_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Ends an active attempt as `state` (merged or abandoned) with its event. Returns its ticket's id, or None when
    /// there is no such active attempt (already ended, or unknown).
    pub async fn end_attempt(
        &self,
        id: &str,
        state: AttemptState,
        actor: Actor,
        now: i64,
    ) -> Result<Option<String>, StoreError> {
        let kind = match state {
            AttemptState::Merged => EventKind::Merged,
            AttemptState::Abandoned => EventKind::Abandoned,
            AttemptState::Active => return Ok(None),
        };
        let mut tx = self.pool.begin().await?;
        let row: Option<(String, String, String)> = sqlx::query_as(
            "UPDATE attempts SET state = ?, ended_at = ? WHERE id = ? AND state = 'active' \
             RETURNING ticket_id, branch, base_branch",
        )
        .bind(state.as_str())
        .bind(now)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((ticket_id, branch, base_branch)) = row else { return Ok(None) };
        let data = json!({"attemptId": id, "branch": branch, "baseBranch": base_branch});
        insert_event(&mut tx, &ticket_id, kind, actor, None, Some(data), now).await?;
        touch(&mut tx, &ticket_id, now).await?;
        tx.commit().await?;
        Ok(Some(ticket_id))
    }

    /// Remembers the attempt's branch head (merge detection), and that the branch has commits of its own once
    /// `own_commits` is true (never cleared).
    pub async fn set_attempt_head(&self, id: &str, head: &str, own_commits: bool) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE attempts SET last_head = ?, own_commits = MAX(own_commits, ?) WHERE id = ?")
            .bind(head)
            .bind(own_commits)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Remembers the workspace Start made for the attempt.
    pub async fn set_attempt_workspace(&self, id: &str, workspace_id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE attempts SET workspace_id = ? WHERE id = ?")
            .bind(workspace_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn get_attempt(&self, id: &str) -> Result<Option<AttemptInfo>, StoreError> {
        let sql = format!("{ATTEMPT_INFO_SELECT} WHERE a.id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_attempt_info).transpose()
    }

    /// The ticket's active attempt, if any.
    pub async fn active_attempt(&self, ticket_id: &str) -> Result<Option<AttemptInfo>, StoreError> {
        let sql = format!("{ATTEMPT_INFO_SELECT} WHERE a.ticket_id = ? AND a.state = 'active'");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(ticket_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_attempt_info).transpose()
    }

    /// Every active attempt, oldest first.
    pub async fn list_active_attempts(&self) -> Result<Vec<AttemptInfo>, StoreError> {
        let sql = format!("{ATTEMPT_INFO_SELECT} WHERE a.state = 'active' ORDER BY a.created_at, a.id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_attempt_info).collect()
    }

    /// The open tickets that take part in overlap detection, in projects that have an active attempt (all projects,
    /// or just `project`): those with an active attempt and those that list areas.
    pub async fn overlap_rows(&self, project: Option<&str>) -> Result<Vec<OverlapRow>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT t.id, t.project_id, COALESCE(p.key, '') AS pkey, t.number, t.title, t.stage, \
             (SELECT a.id FROM attempts a WHERE a.ticket_id = t.id AND a.state = 'active') AS attempt_id, \
             (SELECT a.worktree_id FROM attempts a WHERE a.ticket_id = t.id AND a.state = 'active') AS worktree_id \
             FROM tickets t JOIN projects p ON p.id = t.project_id \
             WHERE t.stage NOT IN ('done', 'wont_do') \
             AND NOT EXISTS (SELECT 1 FROM tickets c WHERE c.parent_id = t.id) \
             AND EXISTS (SELECT 1 FROM attempts a JOIN tickets o ON o.id = a.ticket_id \
                         WHERE a.state = 'active' AND o.project_id = t.project_id) \
             AND (EXISTS (SELECT 1 FROM ticket_areas ta WHERE ta.ticket_id = t.id) \
                  OR EXISTS (SELECT 1 FROM attempts a WHERE a.ticket_id = t.id AND a.state = 'active'))",
        );
        if let Some(p) = project {
            qb.push(" AND t.project_id = ").push_bind(p.to_string());
        }
        qb.push(" ORDER BY t.project_id, t.number");
        let mut rows = qb
            .build()
            .fetch_all(&mut *conn)
            .await?
            .iter()
            .map(|r| {
                let pkey: String = r.try_get("pkey")?;
                let number = u32::try_from(r.try_get::<i64, _>("number")?).unwrap_or(0);
                Ok(OverlapRow {
                    ticket_id: r.try_get("id")?,
                    project_id: r.try_get("project_id")?,
                    key: format_key(&pkey, number),
                    number,
                    title: r.try_get("title")?,
                    stage: stage_of(&r.try_get::<String, _>("stage")?)?,
                    attempt_id: r.try_get("attempt_id")?,
                    worktree_id: r.try_get("worktree_id")?,
                    areas: vec![],
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let ids: Vec<String> = rows.iter().map(|r| r.ticket_id.clone()).collect();
        let mut areas: HashMap<String, Vec<String>> = HashMap::new();
        for chunk in ids.chunks(500) {
            let mut qb = QueryBuilder::<Sqlite>::new("SELECT ticket_id, area FROM ticket_areas WHERE ticket_id IN ");
            push_in(&mut qb, chunk);
            qb.push(" ORDER BY 1, 2");
            for r in qb.build().fetch_all(&mut *conn).await? {
                areas.entry(r.try_get(0)?).or_default().push(r.try_get(1)?);
            }
        }
        for r in rows.iter_mut() {
            r.areas = areas.remove(&r.ticket_id).unwrap_or_default();
        }
        Ok(rows)
    }

    /// The files of the ticket's latest `overlap` event with `other` (a ticket id), if it has one.
    pub async fn last_overlap_files(&self, ticket_id: &str, other: &str) -> Result<Option<Vec<String>>, StoreError> {
        let rows: Vec<(Option<String>,)> = sqlx::query_as(
            "SELECT data FROM ticket_events WHERE ticket_id = ? AND kind = 'overlap' ORDER BY created_at DESC, id DESC",
        )
        .bind(ticket_id)
        .fetch_all(&self.pool)
        .await?;
        for (data,) in rows {
            let Some(v) = data.and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok()) else { continue };
            if v.get("ticketId").and_then(|t| t.as_str()) == Some(other) {
                let files = v
                    .get("files")
                    .and_then(|f| f.as_array())
                    .map(|a| a.iter().filter_map(|f| f.as_str().map(String::from)).collect::<Vec<_>>());
                return Ok(Some(files.unwrap_or_default()));
            }
        }
        Ok(None)
    }

    /// The keys of the pull-request notices already recorded for an attempt (`pr` events), so each is told once,
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
            .filter_map(|v| v.get("key").and_then(|k| k.as_str()).map(String::from))
            .collect())
    }

    /// Adds an event to a ticket's timeline (e.g. `session` when Start made one).
    pub async fn add_ticket_event(
        &self,
        ticket_id: &str,
        kind: EventKind,
        actor: Actor,
        body: Option<&str>,
        data: Option<serde_json::Value>,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut conn = self.pool.acquire().await?;
        insert_event(&mut conn, ticket_id, kind, actor, body, data, now).await?;
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
            parent: None,
            blocked_by_previous: false,
        }
    }

    fn part(title: &str, parent: &str) -> ValidNewTicket {
        ValidNewTicket { parent: Some(parent.into()), ..new(title) }
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
            ticket: None,
        };
        store.insert_workspace(&free).await.unwrap();
        store.move_tab("ta", Some("f"), None).await.unwrap().unwrap();
        store.delete_workspace("f", false).await.unwrap();
        assert_eq!(store.get_workspace(&home).await.unwrap().unwrap().tabs.len(), 2);

        assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::Deleted);
        let tabs = store.get_workspace(&home).await.unwrap().unwrap().tabs;
        assert_eq!(tabs.iter().map(|t| (t.id.as_str(), t.position)).collect::<Vec<_>>(), [("ta", 0)]);
    }

    fn agent_session(id: &str, attempt: Option<&str>, created_at: i64) -> pdt_core::session::Session {
        use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
        Session {
            id: id.into(),
            project_id: "p".into(),
            worktree_id: None,
            kind: SessionKind::AgentTui,
            agent: Some(AgentKind::Claude),
            bypass_permissions: false,
            title: format!("session {id}"),
            cwd: "/r".into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: None,
            activity: None,
            attention_at: None,
            last_viewed_at: None,
            unseen: false,
            attempt_id: attempt.map(Into::into),
            ticket: None,
            mode: pdt_core::conversation::SessionMode::Terminal,
            model: None,
            permission_mode: None,
        }
    }

    #[tokio::test]
    async fn starting_an_attempt_moves_the_ticket_and_can_be_undone() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let t = add(&store, "a", ValidNewTicket { stage: TicketStage::Ready, ..new("Fix the login") }).await;
        assert_eq!(t.default_branch, "pdt/PX-1-fix-the-login");
        let before = store.get_ticket("a").await.unwrap().unwrap();

        let started = store
            .start_attempt("at1", "a", None, "pdt/PX-1-fix-the-login", "main", "abc", json!({"forced": true}), 5)
            .await
            .unwrap();
        assert_eq!((started.attempt.state, started.attempt.last_head.as_deref()), (AttemptState::Active, None));
        let t = store.get_ticket("a").await.unwrap().unwrap();
        assert_eq!(t.summary.stage, TicketStage::InProgress);
        assert_eq!(t.attempts.len(), 1);
        assert_eq!(events(&store, "a").await, [EventKind::Created, EventKind::Moved, EventKind::Started]);
        let page = store.list_events("a", None, 10, true).await.unwrap().unwrap();
        let data = page.items[0].data.clone().unwrap();
        assert_eq!((data["attemptId"].as_str(), data["forced"].as_bool()), (Some("at1"), Some(true)));

        // One active attempt per ticket.
        let again = store.start_attempt("at2", "a", None, "b", "main", "abc", json!({}), 6).await;
        assert!(matches!(again, Err(TicketError::Active(k)) if k == "PX-1"));
        assert!(matches!(
            store.start_attempt("at2", "nope", None, "b", "main", "abc", json!({}), 6).await,
            Err(TicketError::NotFound("ticket"))
        ));

        // Undo puts everything back.
        store.undo_start(&started).await.unwrap();
        let t = store.get_ticket("a").await.unwrap().unwrap();
        assert_eq!((t.summary.stage, t.summary.updated_at), (TicketStage::Ready, before.summary.updated_at));
        assert!(t.attempts.is_empty());
        assert_eq!(events(&store, "a").await, [EventKind::Created]);

        // A ticket already in progress gets no `moved` event.
        add(&store, "b", ValidNewTicket { stage: TicketStage::InProgress, ..new("B") }).await;
        store.start_attempt("bt1", "b", None, "x", "main", "abc", json!({}), 7).await.unwrap();
        assert_eq!(events(&store, "b").await, [EventKind::Created, EventKind::Started]);
    }

    #[tokio::test]
    async fn attempts_end_once_and_remember_their_head() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "a", new("A")).await;
        store.start_attempt("at1", "a", None, "br", "main", "abc", json!({}), 5).await.unwrap();
        let active = store.list_active_attempts().await.unwrap();
        assert_eq!(active.len(), 1);
        let info = &active[0];
        assert_eq!(
            (info.ticket_key.as_str(), info.repo_path.as_str(), info.base_commit.as_str()),
            ("PX-1", "/r", "abc")
        );
        assert_eq!(info.worktree_path, None);
        assert_eq!(store.active_attempt("a").await.unwrap().map(|a| a.attempt.id), Some("at1".into()));

        assert!(!info.own_commits);
        assert!(store.set_attempt_head("at1", "def", false).await.unwrap());
        let info = store.get_attempt("at1").await.unwrap().unwrap();
        assert_eq!((info.attempt.last_head.as_deref(), info.own_commits), (Some("def"), false));
        assert!(store.set_attempt_head("at1", "ghi", true).await.unwrap());
        assert!(store.set_attempt_head("at1", "def", false).await.unwrap());
        let info = store.get_attempt("at1").await.unwrap().unwrap();
        assert_eq!((info.attempt.last_head.as_deref(), info.own_commits), (Some("def"), true), "own commits stick");

        // The workspace Start made is remembered, and forgotten when the workspace is deleted.
        assert_eq!(store.get_attempt("at1").await.unwrap().unwrap().attempt.workspace_id, None);
        let w = pdt_core::workspace::Workspace {
            id: "ws".into(),
            name: "PX-1".into(),
            project_id: None,
            position: 0,
            created_at: 5,
            tabs: vec![],
            ticket: None,
        };
        store.insert_workspace(&w).await.unwrap();
        assert!(store.set_attempt_workspace("at1", "ws").await.unwrap());
        assert_eq!(store.get_attempt("at1").await.unwrap().unwrap().attempt.workspace_id.as_deref(), Some("ws"));
        assert_eq!(store.get_ticket("a").await.unwrap().unwrap().attempts[0].workspace_id.as_deref(), Some("ws"));
        store.delete_workspace("ws", false).await.unwrap();
        assert_eq!(store.get_attempt("at1").await.unwrap().unwrap().attempt.workspace_id, None);

        assert_eq!(
            store.end_attempt("at1", AttemptState::Merged, Actor::System, 9).await.unwrap().as_deref(),
            Some("a")
        );
        assert_eq!(store.end_attempt("at1", AttemptState::Abandoned, Actor::User, 10).await.unwrap(), None);
        let a = store.get_attempt("at1").await.unwrap().unwrap().attempt;
        assert_eq!((a.state, a.ended_at), (AttemptState::Merged, Some(9)));
        assert!(store.list_active_attempts().await.unwrap().is_empty());
        assert!(store.active_attempt("a").await.unwrap().is_none());
        assert_eq!(
            events(&store, "a").await,
            [EventKind::Created, EventKind::Moved, EventKind::Started, EventKind::Merged]
        );
        // A new attempt can start after the last one ended.
        store.start_attempt("at2", "a", None, "br", "main", "abc", json!({}), 11).await.unwrap();
        assert_eq!(
            store.end_attempt("at2", AttemptState::Abandoned, Actor::User, 12).await.unwrap().as_deref(),
            Some("a")
        );
        assert_eq!(events(&store, "a").await.last(), Some(&EventKind::Abandoned));
    }

    #[tokio::test]
    async fn sessions_carry_their_ticket_and_tickets_list_their_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "a", new("A")).await;
        store.start_attempt("at1", "a", None, "br", "main", "abc", json!({}), 5).await.unwrap();
        store.insert_session(&agent_session("s1", Some("at1"), 1)).await.unwrap();
        store.insert_session(&agent_session("s2", Some("at1"), 2)).await.unwrap();
        store.insert_session(&agent_session("plain", None, 3)).await.unwrap();

        let s = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!(s.attempt_id.as_deref(), Some("at1"));
        let t = s.ticket.unwrap();
        assert_eq!((t.id.as_str(), t.key.as_str()), ("a", "PX-1"));
        let plain = store.get_session("plain").await.unwrap().unwrap();
        assert_eq!((plain.attempt_id, plain.ticket), (None, None));
        let listed = store.list_sessions("p").await.unwrap();
        assert_eq!(listed.iter().filter(|s| s.ticket.is_some()).count(), 2);

        let ticket = store.get_ticket("a").await.unwrap().unwrap();
        let ids: Vec<&str> = ticket.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s2", "s1"], "newest first");
        assert_eq!(ticket.sessions[0].attempt_id, "at1");

        // A chat joins an attempt once; a session with one keeps it.
        assert!(store.set_session_attempt("plain", "at1").await.unwrap());
        assert!(!store.set_session_attempt("plain", "at1").await.unwrap());
        assert!(!store.set_session_attempt("s1", "at1").await.unwrap());
        assert!(!store.set_session_attempt("gone", "at1").await.unwrap());
        let ticket = store.get_ticket("a").await.unwrap().unwrap();
        assert_eq!(ticket.sessions.len(), 3);
        store.delete_session("plain").await.unwrap();

        // Deleting the ticket (here directly: the API refuses while the attempt is active) drops the attempt; its
        // sessions lose the link.
        assert_eq!(store.delete_ticket("a").await.unwrap(), DeleteTicket::Active);
        sqlx::query("DELETE FROM tickets WHERE id = 'a'").execute(&store.pool).await.unwrap();
        let s = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!((s.attempt_id, s.ticket), (None, None));
    }

    #[tokio::test]
    async fn closed_after_keeps_open_tickets_and_recently_closed_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "open", new("Open")).await;
        add(&store, "old", new("Old")).await;
        add(&store, "recent", new("Recent")).await;
        let done = ValidPatch { stage: Some(TicketStage::Done), ..Default::default() };
        store.patch_ticket("old", &done, 100).await.unwrap();
        store.patch_ticket("recent", &done, 200).await.unwrap();
        let mut got = ids(&store, TicketQuery { closed_after: Some(150), ..in_project() }).await;
        got.sort();
        assert_eq!(got, ["open", "recent"]);
        let mut got = ids(&store, TicketQuery { closed_after: Some(100), ..in_project() }).await;
        got.sort();
        assert_eq!(got, ["old", "open", "recent"]);
    }

    fn patch() -> ValidPatch {
        ValidPatch::default()
    }

    #[tokio::test]
    async fn waiting_goes_to_the_top_and_ends_when_started_or_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "a", ValidNewTicket { stage: TicketStage::Ready, ..new("A") }).await;
        add(&store, "b", ValidNewTicket { stage: TicketStage::Ready, ..new("B") }).await;
        assert_eq!(ids(&store, in_project()).await, ["b", "a"]);

        let t = store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 5).await.unwrap().unwrap();
        assert!(t.summary.waiting);
        assert_eq!(ids(&store, in_project()).await, ["a", "b"], "a waiting ticket sits at the top of its stage");
        assert_eq!(events(&store, "a").await.last(), Some(&EventKind::Waiting));
        // Setting it again changes nothing.
        store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 6).await.unwrap();
        assert_eq!(events(&store, "a").await.iter().filter(|e| **e == EventKind::Waiting).count(), 1);
        let t = store.patch_ticket("a", &ValidPatch { waiting: Some(false), ..patch() }, 7).await.unwrap().unwrap();
        assert!(!t.summary.waiting);

        store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 8).await.unwrap();
        store.start_attempt("at1", "a", None, "x", "main", "abc", json!({}), 9).await.unwrap();
        assert!(!store.get_ticket_summary("a").await.unwrap().unwrap().waiting, "started");
        let r = store.patch_ticket("a", &ValidPatch { waiting: Some(true), ..patch() }, 10).await;
        assert!(matches!(r, Err(TicketError::Invalid(_))), "an active ticket can't wait");

        store.patch_ticket("b", &ValidPatch { waiting: Some(true), ..patch() }, 11).await.unwrap();
        let closed = ValidPatch { stage: Some(TicketStage::WontDo), ..patch() };
        let t = store.patch_ticket("b", &closed, 12).await.unwrap().unwrap();
        assert!(!t.summary.waiting, "closed");
        let r = store.patch_ticket("b", &ValidPatch { waiting: Some(true), ..patch() }, 13).await;
        assert!(matches!(r, Err(TicketError::Invalid(_))), "a closed ticket can't wait");
    }

    #[tokio::test]
    async fn id_filters_narrow_the_list_and_its_total() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        for id in ["a", "b", "c"] {
            add(&store, id, new(id)).await;
        }
        let mut f = filter(in_project());
        f.only_ids = Some(vec!["a".into(), "c".into()]);
        let page = store.query_tickets(&f).await.unwrap();
        assert_eq!((page.total, page.items.len()), (2, 2));
        f.only_ids = Some(vec![]);
        assert_eq!(store.query_tickets(&f).await.unwrap().total, 0);
        f.only_ids = None;
        f.except_ids = vec!["b".into()];
        let got: Vec<String> = store.query_tickets(&f).await.unwrap().items.into_iter().map(|t| t.id).collect();
        assert_eq!(got, ["c", "a"]);
    }

    #[tokio::test]
    async fn overlap_rows_are_open_tickets_with_areas_or_attempts_where_work_is_active() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let areas = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        add(&store, "a", ValidNewTicket { areas: areas(&["web", "db"]), ..new("A") }).await;
        add(&store, "b", ValidNewTicket { areas: areas(&["web"]), ..new("B") }).await;
        add(&store, "c", new("C")).await;
        add(&store, "d", ValidNewTicket { areas: areas(&["web"]), stage: TicketStage::Done, ..new("D") }).await;
        assert!(store.overlap_rows(None).await.unwrap().is_empty(), "nothing is active");

        store.start_attempt("at1", "c", None, "x", "main", "abc", json!({}), 5).await.unwrap();
        let rows = store.overlap_rows(Some("p")).await.unwrap();
        let got: Vec<(&str, Option<&str>, Vec<String>)> =
            rows.iter().map(|r| (r.ticket_id.as_str(), r.attempt_id.as_deref(), r.areas.clone())).collect();
        assert_eq!(got, [("a", None, areas(&["db", "web"])), ("b", None, areas(&["web"])), ("c", Some("at1"), vec![])]);
        assert_eq!(rows[2].key, "PX-3");
        assert!(store.overlap_rows(Some("other")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_last_overlap_event_with_a_ticket_is_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "a", new("A")).await;
        assert_eq!(store.last_overlap_files("a", "b").await.unwrap(), None);
        let ev = |other: &str, files: &[&str], now| {
            let data = json!({"ticketId": other, "key": "PX-9", "files": files, "level": "overlap"});
            let store = store.clone();
            async move { store.add_ticket_event("a", EventKind::Overlap, Actor::System, None, Some(data), now).await }
        };
        ev("b", &["x"], 2).await.unwrap();
        ev("c", &["y"], 3).await.unwrap();
        ev("b", &["x", "z"], 4).await.unwrap();
        assert_eq!(store.last_overlap_files("a", "b").await.unwrap(), Some(vec!["x".into(), "z".into()]));
        assert_eq!(store.last_overlap_files("a", "c").await.unwrap(), Some(vec!["y".into()]));
    }

    #[tokio::test]
    async fn parts_follow_their_parent_in_order_and_can_block_each_other() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "other", new("Other")).await;
        add(&store, "par", new("Parent")).await;
        add(&store, "top", new("Top")).await;
        let a = add(&store, "a", part("5a", "par")).await;
        assert_eq!(a.summary.parent.as_ref().map(|p| p.key.as_str()), Some("PX-2"));
        let b = store
            .create_ticket("b", "p", &ValidNewTicket { blocked_by_previous: true, ..part("5b", "par") }, 2)
            .await
            .unwrap();
        assert_eq!(b.summary.blocked_by, 1, "5a blocks 5b");
        // Parts sit right after the parent, in the order made.
        assert_eq!(ids(&store, in_project()).await, ["top", "par", "a", "b", "other"]);
        let parent = store.get_ticket("par").await.unwrap().unwrap();
        assert_eq!(parent.part_list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(events(&store, "par").await.contains(&EventKind::Parted));
        assert!(events(&store, "a").await.contains(&EventKind::Parted));
        // Filters.
        let only = |q: TicketQuery| ids(&store, TicketQuery { project: Some("p".into()), ..q });
        assert_eq!(only(TicketQuery { parent: Some("par".into()), ..Default::default() }).await, ["a", "b"]);
        assert_eq!(
            only(TicketQuery { parent: Some("none".into()), ..Default::default() }).await,
            ["top", "par", "other"]
        );
        assert_eq!(only(TicketQuery { has_parts: Some(true), ..Default::default() }).await, ["par"]);
        assert_eq!(only(TicketQuery { has_parts: Some(false), ..Default::default() }).await.len(), 4);
        // Several at once, after the existing ones, each blocked by the one before.
        let v = ValidParts {
            parts: vec![("5c".into(), "c".into()), ("5d".into(), String::new())],
            blocked_by_previous: true,
        };
        let made = store.create_parts("par", &v, 3).await.unwrap();
        assert_eq!(made.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["5c", "5d"]);
        assert!(made.iter().all(|t| t.blocked_by == 1));
        let rows = store.part_rows(&["par".into()]).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["5a", "5b", "5c", "5d"]);
        assert_eq!(rows.iter().map(|r| r.open_blockers).collect::<Vec<_>>(), [0, 1, 1, 1]);
        assert!(rows.iter().all(|r| r.attempt.is_none() && r.parent_id == "par"));
    }

    #[tokio::test]
    async fn the_parent_rules_hold_in_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "par", new("Parent")).await;
        add(&store, "a", part("A", "par")).await;
        add(&store, "x", new("X")).await;
        let err = |r: TResult<Option<Ticket>>| match r {
            Err(TicketError::InvalidParent(rule)) => rule,
            other => panic!("{other:?}"),
        };
        use pdt_core::parts::ParentRule;
        let to = |p: &str| ValidPatch { parent: Some(Some(p.into())), ..Default::default() };
        assert_eq!(err(store.patch_ticket("x", &to("a"), 2).await), ParentRule::ParentIsAPart);
        assert_eq!(err(store.patch_ticket("par", &to("x"), 2).await), ParentRule::ChildHasParts);
        assert_eq!(err(store.patch_ticket("x", &to("x"), 2).await), ParentRule::Itself);
        assert!(matches!(
            store.create_ticket("y", "p", &part("Y", "a"), 2).await,
            Err(TicketError::InvalidParent(ParentRule::ParentIsAPart))
        ));
        store.start_attempt("at", "x", None, "b", "main", "c", json!({}), 3).await.unwrap();
        assert!(matches!(
            store.create_ticket("y", "p", &part("Y", "x"), 2).await,
            Err(TicketError::InvalidParent(ParentRule::ParentActive))
        ));
        // Move a ticket in, then detach it.
        add(&store, "z", new("Z")).await;
        let z = store.patch_ticket("z", &to("par"), 4).await.unwrap().unwrap();
        assert_eq!(z.summary.parent.map(|p| p.id), Some("par".into()));
        let z = store
            .patch_ticket("z", &ValidPatch { parent: Some(None), ..Default::default() }, 5)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(z.summary.parent, None);
        assert_eq!(events(&store, "z").await.iter().filter(|e| **e == EventKind::Parted).count(), 2);
    }

    #[tokio::test]
    async fn a_parent_closes_only_after_its_parts_and_is_not_deleted_with_them() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "par", new("Parent")).await;
        add(&store, "a", part("A", "par")).await;
        add(&store, "b", part("B", "par")).await;
        let done = ValidPatch { stage: Some(TicketStage::Done), ..Default::default() };
        match store.patch_ticket("par", &done, 2).await {
            Err(TicketError::PartsOpen { key, open }) => {
                assert_eq!((key.as_str(), open), ("PX-1", vec!["PX-2".to_string(), "PX-3".to_string()]))
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(store.delete_ticket("par").await.unwrap(), DeleteTicket::HasParts);
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(matches!(
            store.bulk_tickets(&ids(&["par", "a"]), &ValidBulk::Stage(TicketStage::Done), 3).await,
            Err(TicketError::PartsOpen { .. })
        ));
        assert!(matches!(
            store.bulk_tickets(&ids(&["par", "a"]), &ValidBulk::Delete, 3).await,
            Err(TicketError::HasParts(_))
        ));
        // Closing the parent with all its parts in one bulk move works; so does a move once they are closed.
        store.bulk_tickets(&ids(&["a"]), &ValidBulk::Stage(TicketStage::WontDo), 4).await.unwrap();
        store.bulk_tickets(&ids(&["par", "b"]), &ValidBulk::Stage(TicketStage::Done), 5).await.unwrap();
        assert_eq!(store.get_ticket("par").await.unwrap().unwrap().summary.stage, TicketStage::Done);
        // Deleting a parent with all its parts at once is fine, and removing the project still works with parts.
        store.bulk_tickets(&ids(&["par", "a", "b"]), &ValidBulk::Delete, 6).await.unwrap();
        add(&store, "p2", new("P2")).await;
        add(&store, "c", part("C", "p2")).await;
        assert!(store.delete_project("p").await.unwrap());
    }

    #[tokio::test]
    async fn a_part_starting_moves_its_parent_once() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "par", ValidNewTicket { stage: TicketStage::Todo, ..new("Parent") }).await;
        add(&store, "a", part("A", "par")).await;
        assert!(store.part_started("par", 2).await.unwrap());
        assert!(!store.part_started("par", 3).await.unwrap());
        assert_eq!(store.get_ticket("par").await.unwrap().unwrap().summary.stage, TicketStage::InProgress);

        store.start_attempt("at", "a", None, "b", "main", "c", json!({}), 4).await.unwrap();
        let rows = store.part_rows(&["par".into()]).await.unwrap();
        assert_eq!(rows[0].attempt, Some(("at".into(), AttemptState::Active)));
    }

    #[tokio::test]
    async fn parts_copy_their_parents_areas_and_parents_have_no_overlap() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        add(&store, "par", ValidNewTicket { areas: vec!["web".into()], ..new("Parent") }).await;
        let a = add(&store, "a", part("A", "par")).await;
        assert_eq!(a.summary.areas, ["web"]);
        let b = add(&store, "b", ValidNewTicket { areas: vec!["api".into()], ..part("B", "par") }).await;
        assert_eq!(b.summary.areas, ["api"], "given areas win");
        store.start_attempt("at", "a", None, "br", "main", "c", json!({}), 2).await.unwrap();
        let rows = store.overlap_rows(Some("p")).await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.ticket_id.as_str()).collect();
        assert!(!ids.contains(&"par"), "a parent is not compared: {ids:?}");
        assert!(ids.contains(&"a") && ids.contains(&"b"));
    }
}
