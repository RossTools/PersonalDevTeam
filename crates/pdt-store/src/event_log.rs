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

/// Adds `e` (already redacted), logged at `now`: the new event, or None for a duplicate.
pub(crate) async fn emit_now(store: &Store, e: &NewLogEvent, now: i64) -> Result<Option<LogEvent>, StoreError> {
    let mut conn = store.pool.acquire().await?;
    if !emit_in(&mut conn, e, now).await? {
        return Ok(None);
    }
    let sql = format!("SELECT {COLUMNS} FROM events WHERE type = ? AND key = ?");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&e.kind).bind(&e.key).fetch_one(&mut *conn).await?;
    to_event(&row).map(Some)
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
    /// Adds an event unless one with its key is logged: the new event, or None for a duplicate. Known secrets' values
    /// are redacted from its payload (PDT-158). One that can't be written (a full disk) is kept in memory and logged
    /// once writes work again (`log_pending_events`, PDT-323), and the error is returned.
    pub async fn emit_event(&self, e: &NewLogEvent, now: i64) -> Result<Option<LogEvent>, StoreError> {
        let redacted;
        let e = if self.redactor.is_empty() {
            e
        } else {
            redacted = NewLogEvent { payload: self.redactor.json(&e.payload), ..e.clone() };
            &redacted
        };
        match self.noted(emit_now(self, e, now).await) {
            Err(err @ StoreError::Write(_)) => {
                self.keep_pending_event(e, now);
                Err(err)
            }
            r => r,
        }
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

/// Mirrors a ticket timeline event into the log, in the same transaction (Phase 7a spec §2.1): `created`, `moved`,
/// `unblocked`, `comment`, `started`, `merged`, `abandoned`, `overlap` (found or cleared) and `outcome` become
/// `ticket.*`, `attempt.*` and `agent.*` events.
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
    if !matches!(
        kind,
        K::Created
            | K::Moved
            | K::Unblocked
            | K::Comment
            | K::Started
            | K::Merged
            | K::Abandoned
            | K::Overlap
            | K::Outcome
    ) {
        return Ok(());
    }
    let head: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT t.project_id, COALESCE(p.key, ''), t.number, t.title FROM tickets t JOIN projects p ON p.id = t.project_id \
         WHERE t.id = ?",
    )
    .bind(ticket_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((project_id, pkey, number, ticket_title)) = head else { return Ok(()) };
    let ticket_key = pdt_core::ticket::format_key(&pkey, u32::try_from(number).unwrap_or(0));
    let null = serde_json::Value::Null;
    let data = data.unwrap_or(&null);
    let s = |k: &str| data.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    // The first issue of each other tracker it is linked to, by system (`github` also as `github_issue`): those Done
    // closes, so the trackers' Done built-ins leave an issue linked by hand alone unless the user asked (PDT-481).
    let external: std::collections::BTreeMap<String, String> = if kind == K::Moved {
        sqlx::query_as::<_, (String, String)>(
            "SELECT system, MIN(key) FROM ticket_links_external WHERE ticket_id = ? AND close_on_done = 1 \
             GROUP BY system",
        )
        .bind(ticket_id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .collect()
    } else {
        Default::default()
    };
    let event = match kind {
        K::Created => {
            let Some(t) = crate::tickets::summary(&mut *conn, ticket_id).await? else { return Ok(()) };
            let agent = pdt_core::ticket::AgentBy::of_event(kind, actor, Some(data));
            let by = pdt_core::ticket::By { actor, agent: agent.clone(), confirmed: false, proposed_by: None };
            NewLogEvent::new(
                ev::TICKET_CREATED,
                timeline_id,
                "tickets",
                ev::TicketCreated {
                    project_id,
                    ticket_id: ticket_id.into(),
                    ticket_key,
                    title: t.title,
                    stage: t.stage.as_str().into(),
                    labels: t.labels,
                    priority: t.priority.as_str().into(),
                    parent_id: t.parent.as_ref().map(|p| p.id.clone()),
                    parent_key: t.parent.map(|p| p.key),
                    actor: pdt_core::ticket::creator(t.source, &by).into(),
                    persona: agent.as_ref().and_then(|a| a.persona.clone()),
                    session_id: agent.as_ref().map(|a| a.session_id.clone()),
                    from_ticket: agent.as_ref().and_then(|a| a.from_ticket.clone()),
                    from_ticket_id: agent.and_then(|a| a.from_ticket_id),
                    via_parts: data.get("viaParts").and_then(|v| v.as_bool()) == Some(true),
                    reason: data.get("reason").and_then(|v| v.as_str()).map(String::from),
                },
            )
        }
        K::Unblocked => {
            let Some(t) = crate::tickets::summary(&mut *conn, ticket_id).await? else { return Ok(()) };
            let blocker_id = s("by");
            let blocker_key = match crate::tickets::summary(&mut *conn, &blocker_id).await? {
                Some(b) => b.key,
                None => String::new(),
            };
            let reason = data.get("reason").and_then(|v| v.as_str()).unwrap_or("closed").to_string();
            NewLogEvent::new(
                ev::TICKET_UNBLOCKED,
                timeline_id,
                "tickets",
                ev::TicketUnblocked {
                    project_id,
                    ticket_id: ticket_id.into(),
                    ticket_key,
                    title: t.title,
                    stage: t.stage.as_str().into(),
                    priority: t.priority.as_str().into(),
                    labels: t.labels,
                    blocker_id,
                    blocker_key,
                    reason,
                },
            )
        }
        K::Moved => {
            let parent = crate::tickets::summary(&mut *conn, ticket_id).await?.and_then(|t| t.parent);
            NewLogEvent::new(
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
                    github_issue: external.get("github").cloned(),
                    external,
                    parent_id: parent.as_ref().map(|p| p.id.clone()),
                    parent_key: parent.map(|p| p.key),
                },
            )
        }
        K::Comment => {
            let attempt: Option<(String,)> =
                sqlx::query_as("SELECT id FROM attempts WHERE ticket_id = ? AND state = 'active'")
                    .bind(ticket_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            let body = body.unwrap_or_default();
            let attachments = crate::attachments::referenced(&mut *conn, ticket_id, body)
                .await?
                .into_iter()
                .map(|a| ev::CommentAttachment {
                    file_name: a.file_name(),
                    id: a.id,
                    name: a.name,
                    media_type: a.media_type,
                    path: None,
                })
                .collect();
            // The user, or the persona of the agent session that wrote it (the PM's comments reach the working agent:
            // PDT-341).
            let author = match data.pointer("/agent/persona").and_then(|p| p.as_str()) {
                Some(persona) if actor != pdt_core::ticket::Actor::User => persona.to_string(),
                _ => actor.as_str().to_string(),
            };
            NewLogEvent::new(
                ev::TICKET_COMMENTED,
                timeline_id,
                "tickets",
                ev::TicketCommented {
                    project_id,
                    ticket_id: ticket_id.into(),
                    ticket_key,
                    body: body.into(),
                    actor: actor.as_str().into(),
                    attempt_id: attempt.map(|(a,)| a),
                    workflow_id: data.get("workflowId").and_then(|w| w.as_str()).map(String::from),
                    by_session: data
                        .pointer("/agent/sessionId")
                        .or_else(|| data.get("sessionId"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    author,
                    attachments,
                    attachment_list: String::new(),
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
        K::Overlap if data.get("cleared").and_then(|c| c.as_bool()) == Some(true) => NewLogEvent::new(
            ev::TICKET_OVERLAP_CLEARED,
            timeline_id,
            "overlap",
            ev::TicketOverlapCleared {
                project_id,
                ticket_id: ticket_id.into(),
                ticket_key,
                other_ticket_id: s("ticketId"),
                other_key: s("key"),
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
                ticket_title,
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

    /// PDT-242: CI events logged before CI providers get text run ids, `name` for `workflow`, and their provider.
    #[tokio::test]
    async fn the_ci_providers_migration_rewrites_old_ci_events() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("m");
        std::fs::create_dir_all(&old).unwrap();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for e in std::fs::read_dir(&src).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with("_ci_run_providers.sql") {
                std::fs::copy(e.path(), old.join(&name)).unwrap();
            }
        }
        let (db, backups) = (tmp.path().join("pdt.db"), tmp.path().join("backups"));
        let migrator = sqlx::migrate::Migrator::new(old.as_path()).await.unwrap();
        let store = Store::open_with(&db, &migrator, &backups).await.unwrap();
        let runs =
            json!([{"id": 391, "attempt": 2, "workflow": "ci", "state": "failed", "url": "u", "failedJobs": ["t"]}]);
        for (kind, key, provider) in [("ci.run_failed", "k1", "github_actions"), ("ci.run_passed", "k2", "buildkite")] {
            let payload = json!({"provider": provider, "runs": runs, "headSha": "h"});
            store.emit_event(&NewLogEvent::new(kind, key, "test", payload), 1).await.unwrap();
        }
        store.emit_event(&NewLogEvent::new("pr.conflict", "k3", "test", json!({"runs": runs})), 1).await.unwrap();
        store.close().await;

        let store = Store::open_with(&db, &crate::migrate::MIGRATOR, &backups).await.unwrap();
        let pool = store.pool.clone();
        let payload = |key: &'static str| {
            let pool = pool.clone();
            async move {
                let (p,): (String,) = sqlx::query_as("SELECT payload FROM events WHERE key = ?")
                    .bind(key)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                serde_json::from_str::<serde_json::Value>(&p).unwrap()
            }
        };
        let p = payload("k1").await;
        assert_eq!(
            p["runs"],
            json!([{"provider": "github_actions", "id": "391", "attempt": 2, "name": "ci", "state": "failed",
                "url": "u", "failedJobs": ["t"]}])
        );
        assert_eq!(p["headSha"], "h", "the rest stays");
        assert_eq!(payload("k2").await["runs"][0]["provider"], "buildkite");
        assert_eq!(payload("k3").await["runs"][0]["id"], 391, "other events are left alone");
    }
}
