//! Tickets (tickets spec §3, §4): numbering, ranks, the backlog query (filters, full-text search, keyset paging),
//! edits with their timeline events, bulk actions, links and comments.

use crate::{Store, StoreError};
use pdt_core::conversation::SessionMode;
use pdt_core::links::would_cycle;
use pdt_core::overlap::Reported;
use pdt_core::parts::{ParentFacts, TicketParent, ValidParts, open_parts, valid_parent};
use pdt_core::rank;
use pdt_core::session::{AgentKind, SessionStatus};
use pdt_core::start::attempt_branch;
use pdt_core::ticket::{
    Actor, AgentBy, AttemptSession, AttemptState, AttemptSummary, By, EventKind, EventPage, ExternalLink, KeyRef,
    LinkDirection, LinkKind, LinkedTicket, MAX_LABELS, OverlapLevel, ParentFilter, PipelineStage, Priority, RankEdge,
    Ticket, TicketEvent, TicketFilter, TicketPage, TicketSort, TicketSource, TicketStage, TicketSummary, ValidBulk,
    ValidNewTicket, ValidPatch, format_key, parse_key,
};
use serde_json::json;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};
use std::collections::{HashMap, HashSet};

mod attempts;
mod create;
mod events;
mod links;
mod order;
mod overlap;
mod query;
#[cfg(test)]
mod tests;
mod update;

pub use attempts::AttemptInfo;
pub use create::{Made, PartRow};
pub use links::StoredExternalLink;
pub use overlap::OverlapRow;

// Helpers the submodules share (each submodule does `use super::*`).
use create::{insert_ticket, parent_facts, parted_events};
use events::{insert_event, to_event};
use links::link_events;
use order::{Place, rank_for, rebalance, set_rank};
use update::{admit, change_stage, replace_set};

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
    /// An external ticket can't go on the board this way (PDT-201's Todo gate): why.
    #[error("{0}")]
    Untrusted(String),
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
    prev_pipeline_stage: Option<PipelineStage>,
    prev_updated_at: i64,
    event_ids: Vec<String>,
}

/// Priority as a number (higher is more urgent), for sorting.
const PRIORITY_WEIGHT: &str = "(CASE t.priority WHEN 'urgent' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 \
                               WHEN 'low' THEN 1 ELSE 0 END)";
/// How many open tickets block `t`.
const BLOCKED_BY: &str = "(SELECT COUNT(*) FROM ticket_links l JOIN tickets b ON b.id = l.from_id \
                          WHERE l.to_id = t.id AND l.kind = 'blocks' AND b.stage NOT IN ('done', 'wont_do'))";
/// Whether `t` is a ticket proposal waiting for the user (PDT-366): held from its making until it leaves Draft.
const PROPOSED: &str = "(t.proposed_at IS NOT NULL)";
/// Whether `t` waits in the Inbox as a proposal (PDT-366): lists leave it out. A held ticket the workflow never put in
/// the Inbox (it was off, its conditions didn't match, its run failed) stays listed, in Draft, for the user to decide.
const IN_INBOX: &str = "EXISTS (SELECT 1 FROM inbox_items i WHERE i.ticket_id = t.id AND i.kind = 'ticket_proposal' \
                        AND i.status = 'open')";
const FROM: &str =
    " FROM tickets t JOIN projects p ON p.id = t.project_id LEFT JOIN tickets par ON par.id = t.parent_id";

fn summary_select() -> String {
    format!(
        "SELECT t.id, t.project_id, COALESCE(p.key, '') AS pkey, t.number, t.title, t.stage, t.priority, t.rank, \
         t.waiting, t.created_at, t.updated_at, t.closed_at, {BLOCKED_BY} AS blocked_by, {PRIORITY_WEIGHT} AS pw, \
         t.parent_id, par.number AS parent_number, par.title AS parent_title, t.pipeline_stage, t.source, t.admitted_at, t.created_by, {PROPOSED} AS proposed{FROM}"
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
        pipeline_stage: r.try_get::<Option<String>, _>("pipeline_stage")?.as_deref().and_then(PipelineStage::parse),
        agent: None,
        auto_start_at: None,
        pickup_hold: None,
        pickup_paused: None,
        pickup_failed: None,
        external: vec![],
        source: {
            let s: String = r.try_get("source")?;
            TicketSource::parse(&s).ok_or_else(|| corrupt("ticket source", &s))?
        },
        admitted: r.try_get::<Option<i64>, _>("admitted_at")?.is_some(),
        created_by: r.try_get::<Option<String>, _>("created_by")?.and_then(|j| serde_json::from_str(&j).ok()),
        proposed: r.try_get::<i64, _>("proposed")? != 0,
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
        ci: None,
        merge_update: None,
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
    let mut external: HashMap<String, Vec<ExternalLink>> = HashMap::new();
    for chunk in ids.chunks(500) {
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT ticket_id, system, key, url FROM ticket_links_external WHERE ticket_id IN ",
        );
        push_in(&mut qb, chunk);
        qb.push(" ORDER BY ticket_id, system, key");
        for r in qb.build().fetch_all(&mut *conn).await? {
            let link = ExternalLink { system: r.try_get(1)?, key: r.try_get(2)?, url: r.try_get(3)? };
            external.entry(r.try_get(0)?).or_default().push(link);
        }
    }
    for t in items.iter_mut() {
        t.labels = labels.remove(&t.id).unwrap_or_default();
        t.areas = areas.remove(&t.id).unwrap_or_default();
        t.attempt = attempts.remove(&t.id);
        t.external = external.remove(&t.id).unwrap_or_default();
    }
    Ok(())
}

pub(crate) async fn summary(conn: &mut SqliteConnection, id: &str) -> Result<Option<TicketSummary>, StoreError> {
    let sql = format!("{} WHERE t.id = ?", summary_select());
    let Some(row) = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&mut *conn).await? else {
        return Ok(None);
    };
    let mut items = [to_summary(&row)?];
    decorate(conn, &mut items).await?;
    let [s] = items;
    Ok(Some(s))
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

async fn touch(conn: &mut SqliteConnection, id: &str, now: i64) -> Result<(), StoreError> {
    sqlx::query("UPDATE tickets SET updated_at = ? WHERE id = ?").bind(now).bind(id).execute(&mut *conn).await?;
    Ok(())
}
