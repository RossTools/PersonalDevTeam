//! The event log (Phase 7a spec §2): typed, keyed events, deduplicated by key, read by the workflow engine.

use crate::{Store, StoreError};
use pdt_core::workflow::api::{LogEvent, NewLogEvent};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

const COLUMNS: &str =
    "seq, id, type, key, project_id, ticket_id, attempt_id, session_id, source, payload, created_at, processed_at";

/// How long events are kept.
pub const EVENT_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

fn to_event(r: &SqliteRow) -> Result<LogEvent, StoreError> {
    let payload: String = r.try_get("payload")?;
    Ok(LogEvent {
        seq: r.try_get("seq")?,
        id: r.try_get("id")?,
        kind: r.try_get("type")?,
        key: r.try_get("key")?,
        project_id: r.try_get("project_id")?,
        ticket_id: r.try_get("ticket_id")?,
        attempt_id: r.try_get("attempt_id")?,
        session_id: r.try_get("session_id")?,
        source: r.try_get("source")?,
        payload: serde_json::from_str(&payload).map_err(|e| StoreError::Corrupt(format!("event payload: {e}")))?,
        created_at: r.try_get("created_at")?,
        processed_at: r.try_get("processed_at")?,
    })
}

/// Adds an event unless one of its type with its key is logged; true when added. For callers inside a transaction (the timeline
/// mirror), so the event and the change it reports commit together.
pub(crate) async fn emit_in(conn: &mut SqliteConnection, e: &NewLogEvent, now: i64) -> Result<bool, StoreError> {
    let r = sqlx::query(
        "INSERT INTO events (id, type, key, project_id, ticket_id, attempt_id, session_id, source, payload, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (type, key) DO NOTHING",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&e.kind)
    .bind(&e.key)
    .bind(&e.project_id)
    .bind(&e.ticket_id)
    .bind(&e.attempt_id)
    .bind(&e.session_id)
    .bind(&e.source)
    .bind(e.payload.to_string())
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// What `GET /api/events` filters on.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub kind: Option<String>,
    pub project_id: Option<String>,
    pub ticket_id: Option<String>,
    pub attempt_id: Option<String>,
    /// Only events with a larger `seq`.
    pub after: Option<i64>,
    /// Only events logged at or after this time (ms).
    pub since: Option<i64>,
    /// Newest first (else oldest first).
    pub newest_first: bool,
    pub limit: i64,
}

impl Store {
    /// Adds an event unless one with its key is logged: the new event, or None for a duplicate.
    pub async fn emit_event(&self, e: &NewLogEvent, now: i64) -> Result<Option<LogEvent>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        if !emit_in(&mut conn, e, now).await? {
            return Ok(None);
        }
        let sql = format!("SELECT {COLUMNS} FROM events WHERE type = ? AND key = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&e.kind).bind(&e.key).fetch_one(&mut *conn).await?;
        to_event(&row).map(Some)
    }

    pub async fn get_log_event(&self, id: &str) -> Result<Option<LogEvent>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM events WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_event).transpose()
    }

    pub async fn list_log_events(&self, f: &EventFilter) -> Result<Vec<LogEvent>, StoreError> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!("SELECT {COLUMNS} FROM events WHERE 1 = 1"));
        if let Some(k) = &f.kind {
            qb.push(" AND type = ").push_bind(k.clone());
        }
        if let Some(p) = &f.project_id {
            qb.push(" AND project_id = ").push_bind(p.clone());
        }
        if let Some(t) = &f.ticket_id {
            qb.push(" AND ticket_id = ").push_bind(t.clone());
        }
        if let Some(a) = &f.attempt_id {
            qb.push(" AND attempt_id = ").push_bind(a.clone());
        }
        if let Some(after) = f.after {
            qb.push(" AND seq > ").push_bind(after);
        }
        if let Some(since) = f.since {
            qb.push(" AND created_at >= ").push_bind(since);
        }
        qb.push(if f.newest_first { " ORDER BY seq DESC" } else { " ORDER BY seq" });
        qb.push(" LIMIT ").push_bind(f.limit.clamp(1, 1000));
        let rows = qb.build().fetch_all(&self.pool).await?;
        rows.iter().map(to_event).collect()
    }

    /// Events the engine has not matched to triggers yet, oldest first.
    pub async fn unprocessed_events(&self, limit: i64) -> Result<Vec<LogEvent>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM events WHERE processed_at IS NULL ORDER BY seq LIMIT ?");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(limit).fetch_all(&self.pool).await?;
        rows.iter().map(to_event).collect()
    }

    pub async fn mark_event_processed(&self, seq: i64, now: i64) -> Result<(), StoreError> {
        sqlx::query("UPDATE events SET processed_at = ? WHERE seq = ?").bind(now).bind(seq).execute(&self.pool).await?;
        Ok(())
    }

    /// Deletes events older than `before` (runs keep a copy of their trigger). Returns how many went.
    pub async fn prune_events(&self, before: i64) -> Result<u64, StoreError> {
        let r = sqlx::query("DELETE FROM events WHERE created_at < ? AND processed_at IS NOT NULL")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }
}

/// Mirrors a ticket timeline event into the log, in the same transaction (Phase 7a spec §2.1): `moved`, `comment`,
/// `started`, `merged`, `abandoned`, `overlap` and `outcome` become `ticket.*`, `attempt.*` and `agent.*` events.
/// Other kinds aren't domain events workflows react to.
/// A row just written to `ticket_events`.
pub(crate) struct TimelineRow<'a> {
    pub id: &'a str,
    pub ticket_id: &'a str,
    pub kind: pdt_core::ticket::EventKind,
    pub actor: pdt_core::ticket::Actor,
    pub body: Option<&'a str>,
    pub data: Option<&'a serde_json::Value>,
}

pub(crate) async fn mirror_timeline(
    conn: &mut SqliteConnection,
    row: TimelineRow<'_>,
    now: i64,
) -> Result<(), StoreError> {
    let TimelineRow { id: timeline_id, ticket_id, kind, actor, body, data } = row;
    use pdt_core::ticket::EventKind as K;
    use pdt_core::workflow::events::{self as ev, AttemptEvent, AttemptRef, MergeVia};
    if !matches!(kind, K::Moved | K::Comment | K::Started | K::Merged | K::Abandoned | K::Overlap | K::Outcome) {
        return Ok(());
    }
    let head: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT t.project_id, COALESCE(p.key, ''), t.number FROM tickets t JOIN projects p ON p.id = t.project_id \
         WHERE t.id = ?",
    )
    .bind(ticket_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((project_id, pkey, number)) = head else { return Ok(()) };
    let ticket_key = pdt_core::ticket::format_key(&pkey, u32::try_from(number).unwrap_or(0));
    let null = serde_json::Value::Null;
    let data = data.unwrap_or(&null);
    let s = |k: &str| data.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let (jira_key, github_issue) = if kind == K::Moved {
        let first = |system: &'static str| {
            sqlx::query_as::<_, (String,)>(
                "SELECT key FROM ticket_links_external WHERE ticket_id = ? AND system = ? ORDER BY key LIMIT 1",
            )
            .bind(ticket_id)
            .bind(system)
        };
        let jira = first("jira").fetch_optional(&mut *conn).await?.map(|(k,)| k);
        let github = first("github").fetch_optional(&mut *conn).await?.map(|(k,)| k);
        (jira, github)
    } else {
        (None, None)
    };
    let event = match kind {
        K::Moved => NewLogEvent::new(
            ev::TICKET_STAGE_CHANGED,
            timeline_id,
            "tickets",
            ev::TicketStageChanged {
                project_id,
                ticket_id: ticket_id.into(),
                ticket_key,
                from: s("from"),
                to: s("to"),
                actor: actor.as_str().into(),
                jira_key,
                github_issue,
            },
        ),
        K::Comment => {
            let attempt: Option<(String,)> =
                sqlx::query_as("SELECT id FROM attempts WHERE ticket_id = ? AND state = 'active'")
                    .bind(ticket_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            NewLogEvent::new(
                ev::TICKET_COMMENTED,
                timeline_id,
                "tickets",
                ev::TicketCommented {
                    project_id,
                    ticket_id: ticket_id.into(),
                    ticket_key,
                    body: body.unwrap_or_default().into(),
                    actor: actor.as_str().into(),
                    attempt_id: attempt.map(|(a,)| a),
                    workflow_id: data.get("workflowId").and_then(|w| w.as_str()).map(String::from),
                },
            )
        }
        K::Outcome => NewLogEvent::new(
            ev::AGENT_OUTCOME,
            timeline_id,
            "agents",
            ev::AgentOutcome {
                project_id,
                ticket_id: ticket_id.into(),
                ticket_key,
                attempt_id: data.get("attemptId").and_then(|v| v.as_str()).map(String::from),
                outcome: s("outcome"),
                summary: s("summary"),
                persona: s("persona"),
                persona_version: data
                    .get("personaVersion")
                    .and_then(|v| v.as_u64())
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0),
                by: s("by"),
                session_id: data.get("sessionId").and_then(|v| v.as_str()).map(String::from),
                workflow_id: data.get("workflowId").and_then(|v| v.as_str()).map(String::from),
                run_id: data.get("runId").and_then(|v| v.as_str()).map(String::from),
            },
        ),
        K::Overlap => NewLogEvent::new(
            ev::TICKET_OVERLAP_FOUND,
            timeline_id,
            "overlap",
            ev::TicketOverlapFound {
                project_id,
                ticket_id: ticket_id.into(),
                ticket_key,
                other_ticket_id: s("ticketId"),
                other_key: s("key"),
                files: serde_json::from_value(data.get("files").cloned().unwrap_or_default()).unwrap_or_default(),
                level: s("level"),
            },
        ),
        _ => {
            let attempt_id = s("attemptId");
            let row: Option<(String, String, i64, i64)> = sqlx::query_as(
                "SELECT branch, base_branch, finish_on_merge, finish_delete_branch FROM attempts WHERE id = ?",
            )
            .bind(&attempt_id)
            .fetch_optional(&mut *conn)
            .await?;
            let Some((branch, base_branch, finish, delete_branch)) = row else { return Ok(()) };
            let via: Option<MergeVia> = data.get("via").and_then(|v| serde_json::from_value(v.clone()).ok());
            let pr = data.get("pr").and_then(|v| v.as_u64()).and_then(|n| u32::try_from(n).ok());
            let (kind, key) = match kind {
                K::Started => (ev::ATTEMPT_STARTED, timeline_id.to_string()),
                K::Merged => (ev::ATTEMPT_MERGED, format!("{attempt_id}:merged")),
                _ => (ev::ATTEMPT_ABANDONED, timeline_id.to_string()),
            };
            let notice = (kind == ev::ATTEMPT_MERGED).then(|| pdt_core::pr::merged_notice(pr, &branch, &base_branch));
            let session_id = data.get("sessionId").and_then(|v| v.as_str()).map(String::from);
            let labels: Vec<(String,)> =
                sqlx::query_as("SELECT label FROM ticket_labels WHERE ticket_id = ? ORDER BY label")
                    .bind(ticket_id)
                    .fetch_all(&mut *conn)
                    .await?;
            let payload = AttemptEvent {
                attempt: AttemptRef {
                    project_id,
                    ticket_id: ticket_id.into(),
                    ticket_key,
                    attempt_id,
                    branch,
                    base_branch,
                },
                actor: actor.as_str().into(),
                via: if kind == ev::ATTEMPT_MERGED { via.or(Some(MergeVia::Detected)) } else { None },
                pr,
                finish_on_merge: finish != 0,
                finish_delete_branch: delete_branch != 0,
                session_id,
                notice,
                labels: labels.into_iter().map(|(l,)| l).collect(),
            };
            NewLogEvent::new(kind, key, "tickets", payload)
        }
    };
    emit_in(conn, &event, now).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;
    use serde_json::json;

    #[tokio::test]
    async fn a_key_is_logged_once_and_the_engine_reads_what_is_new() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let e = NewLogEvent::new("pr.conflict", "a1:conflict:h", "test", json!({"ticketId": "t", "pr": 3}));
        let first = store.emit_event(&e, 10).await.unwrap().unwrap();
        assert_eq!(
            (first.kind.as_str(), first.ticket_id.as_deref(), first.payload["pr"].as_i64()),
            ("pr.conflict", Some("t"), Some(3))
        );
        assert!(store.emit_event(&e, 11).await.unwrap().is_none(), "same key: dropped");
        let other = NewLogEvent::new("ci.run_failed", "a1:ci_failed:1.1", "test", json!({}));
        store.emit_event(&other, 12).await.unwrap().unwrap();

        let new = store.unprocessed_events(10).await.unwrap();
        assert_eq!(new.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["pr.conflict", "ci.run_failed"]);
        store.mark_event_processed(new[0].seq, 20).await.unwrap();
        assert_eq!(store.unprocessed_events(10).await.unwrap().len(), 1);

        let f = EventFilter { ticket_id: Some("t".into()), limit: 50, ..Default::default() };
        assert_eq!(store.list_log_events(&f).await.unwrap().len(), 1);
        let f = EventFilter { newest_first: true, limit: 50, ..Default::default() };
        assert_eq!(store.list_log_events(&f).await.unwrap()[0].kind, "ci.run_failed");
        assert_eq!(store.get_log_event(&first.id).await.unwrap().unwrap().key, e.key);

        assert_eq!(store.prune_events(100).await.unwrap(), 1, "only processed events go");
    }
}
