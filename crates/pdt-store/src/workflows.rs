//! Workflows, their versions, runs and steps, per-workflow state, the session outbox and connections (Phase 7a spec
//! §3–§5, §7).

use crate::{Store, StoreError};
use pdt_core::secret::Redactor;
use pdt_core::workflow::api::{
    Author, Connection, Run, RunStatus, Scope, Source, Step, StepStatus, VersionStatus, Workflow, WorkflowVersion,
};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::workflow::template::Template;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite};

/// How long finished runs are kept.
pub const RUN_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

const WF_COLUMNS: &str = "id, name, description, scope, project_id, enabled, builtin, source, repo_file, version, \
                          (SELECT MAX(v.version) FROM workflow_versions v WHERE v.workflow_id = workflows.id \
                           AND v.status = 'proposed') AS proposed, created_at, updated_at";
const RUN_COLUMNS: &str = "r.id, r.workflow_id, w.name AS workflow_name, r.version, r.status, r.run_key, \
                           r.trigger_event_id, r.trigger_node, r.trigger, r.project_id, r.ticket_id, r.attempt_id, \
                           r.parent_run_id, r.parent_step_id, r.error, r.output, r.wake_at, r.started_at, r.resumed_at, \
                           r.ended_at";
const STEP_COLUMNS: &str = "id, run_id, seq, node_id, kind, status, input, output, port, attempts, error, wake_at, \
                            wait, started_at, ended_at";
const MSG_COLUMNS: &str = "id, attempt_id, session_id, ticket_id, dedupe, text, timeline, run_id, step_id, status, \
                           told_session, created_at, resolved_at, by_user, pending_id, reopens";
const CONN_COLUMNS: &str = "id, name, kind, url, config, has_secret, created_at, updated_at, last_error, last_error_at";

fn corrupt(what: &str, e: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(format!("{what}: {e}"))
}

fn json_of(s: Option<String>) -> Value {
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn opt_json(v: &Value) -> Option<String> {
    (!v.is_null()).then(|| v.to_string())
}

fn to_workflow(r: &SqliteRow) -> Result<Workflow, StoreError> {
    let scope: String = r.try_get("scope")?;
    let source: String = r.try_get("source")?;
    Ok(Workflow {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        description: r.try_get("description")?,
        scope: Scope::parse(&scope).ok_or_else(|| corrupt("workflow scope", &scope))?,
        project_id: r.try_get("project_id")?,
        enabled: r.try_get::<i64, _>("enabled")? != 0,
        builtin: r.try_get("builtin")?,
        source: Source::parse(&source).ok_or_else(|| corrupt("workflow source", &source))?,
        repo_file: r.try_get("repo_file")?,
        version: u32::try_from(r.try_get::<i64, _>("version")?).unwrap_or(0),
        proposed: r.try_get::<Option<i64>, _>("proposed")?.and_then(|v| u32::try_from(v).ok()),
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

fn to_template(r: &SqliteRow) -> Result<Template, StoreError> {
    let params: String = r.try_get("params")?;
    let doc: String = r.try_get("doc")?;
    Ok(Template {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        description: r.try_get("description")?,
        builtin: false,
        category: Default::default(),
        params: serde_json::from_str(&params).map_err(|e| corrupt("template params", e))?,
        doc: serde_json::from_str(&doc).map_err(|e| corrupt("template doc", e))?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

fn to_run(r: &SqliteRow) -> Result<Run, StoreError> {
    let status: String = r.try_get("status")?;
    Ok(Run {
        id: r.try_get("id")?,
        workflow_id: r.try_get("workflow_id")?,
        workflow_name: r.try_get("workflow_name")?,
        version: u32::try_from(r.try_get::<i64, _>("version")?).unwrap_or(0),
        status: RunStatus::parse(&status).ok_or_else(|| corrupt("run status", &status))?,
        run_key: r.try_get("run_key")?,
        trigger_event_id: r.try_get("trigger_event_id")?,
        trigger_node: r.try_get("trigger_node")?,
        trigger: json_of(r.try_get("trigger")?),
        project_id: r.try_get("project_id")?,
        ticket_id: r.try_get("ticket_id")?,
        attempt_id: r.try_get("attempt_id")?,
        parent_run_id: r.try_get("parent_run_id")?,
        parent_step_id: r.try_get("parent_step_id")?,
        error: r.try_get("error")?,
        output: json_of(r.try_get("output")?),
        wake_at: r.try_get("wake_at")?,
        started_at: r.try_get("started_at")?,
        resumed_at: r.try_get("resumed_at")?,
        ended_at: r.try_get("ended_at")?,
    })
}

fn to_step(r: &SqliteRow) -> Result<Step, StoreError> {
    let status: String = r.try_get("status")?;
    Ok(Step {
        id: r.try_get("id")?,
        run_id: r.try_get("run_id")?,
        seq: r.try_get("seq")?,
        node_id: r.try_get("node_id")?,
        kind: r.try_get("kind")?,
        status: StepStatus::parse(&status).ok_or_else(|| corrupt("step status", &status))?,
        input: json_of(r.try_get("input")?),
        output: json_of(r.try_get("output")?),
        port: r.try_get("port")?,
        attempts: u32::try_from(r.try_get::<i64, _>("attempts")?).unwrap_or(0),
        error: r.try_get("error")?,
        wake_at: r.try_get("wake_at")?,
        wait: json_of(r.try_get("wait")?),
        started_at: r.try_get("started_at")?,
        ended_at: r.try_get("ended_at")?,
    })
}

/// A message for an agent session (`session.send`), waiting for the session to be ready.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMessage {
    pub id: String,
    /// To the attempt's newest running agent session…
    pub attempt_id: Option<String>,
    /// …or to this session.
    pub session_id: Option<String>,
    pub ticket_id: Option<String>,
    pub dedupe: Option<String>,
    pub text: String,
    /// The timeline record to write once resolved (`{kind, data}`), or null.
    pub timeline: Value,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    /// `waiting`, `sending` (a conversation has it, the agent has not echoed it yet), `told` or `untold`.
    pub status: String,
    pub told_session: Option<String>,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    /// The user sent it (an Inbox answer or nudge): the host's cap on agents running a turn never holds it.
    pub by_user: bool,
    /// While `sending`: the conversation's pending message that carries it (PDT-129).
    pub pending_id: Option<String>,
    /// A conversation that is not running (stopped, interrupted) reopens to take it, as for the user's own messages;
    /// unlike those, the host's cap still holds it (PDT-144).
    pub reopens: bool,
}

fn to_message(r: &SqliteRow) -> Result<SessionMessage, StoreError> {
    Ok(SessionMessage {
        id: r.try_get("id")?,
        attempt_id: r.try_get("attempt_id")?,
        session_id: r.try_get("session_id")?,
        ticket_id: r.try_get("ticket_id")?,
        dedupe: r.try_get("dedupe")?,
        text: r.try_get("text")?,
        timeline: json_of(r.try_get("timeline")?),
        run_id: r.try_get("run_id")?,
        step_id: r.try_get("step_id")?,
        status: r.try_get("status")?,
        told_session: r.try_get("told_session")?,
        created_at: r.try_get("created_at")?,
        resolved_at: r.try_get("resolved_at")?,
        by_user: r.try_get::<i64, _>("by_user")? != 0,
        pending_id: r.try_get("pending_id")?,
        reopens: r.try_get::<i64, _>("reopens")? != 0,
    })
}

fn to_connection(r: &SqliteRow) -> Result<Connection, StoreError> {
    Ok(Connection {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        kind: r.try_get("kind")?,
        url: r.try_get("url")?,
        config: json_of(r.try_get("config")?),
        has_secret: r.try_get::<i64, _>("has_secret")? != 0,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        last_error: r.try_get("last_error")?,
        last_error_at: r.try_get("last_error_at")?,
    })
}

/// The number the workflow's next version gets: one more than any so far (saved, proposed or discarded). None for an
/// unknown workflow.
async fn next_version(tx: &mut sqlx::Transaction<'_, Sqlite>, id: &str) -> Result<Option<i64>, StoreError> {
    let known: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM workflows WHERE id = ?").bind(id).fetch_optional(&mut **tx).await?;
    if known.is_none() {
        return Ok(None);
    }
    let (max,): (Option<i64>,) = sqlx::query_as("SELECT MAX(version) FROM workflow_versions WHERE workflow_id = ?")
        .bind(id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(Some(max.unwrap_or(0) + 1))
}

/// What `GET /api/workflow-runs` filters on.
#[derive(Debug, Clone, Default)]
pub struct RunFilter {
    pub workflow_id: Option<String>,
    pub ticket_id: Option<String>,
    pub status: Option<RunStatus>,
    pub limit: i64,
}

impl Store {
    // ---- workflows and versions ----

    /// Adds a workflow with its first version.
    pub async fn insert_workflow(
        &self,
        w: &Workflow,
        doc: &WorkflowDoc,
        author: Author,
        note: &str,
    ) -> Result<(), StoreError> {
        let body = serde_json::to_string(doc).map_err(|e| corrupt("workflow doc", e))?;
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "INSERT INTO workflows (id, name, description, scope, project_id, enabled, builtin, source, repo_file, \
             version, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(&w.id)
        .bind(&doc.name)
        .bind(&doc.description)
        .bind(w.scope.as_str())
        .bind(&w.project_id)
        .bind(w.enabled)
        .bind(&w.builtin)
        .bind(w.source.as_str())
        .bind(&w.repo_file)
        .bind(w.created_at)
        .bind(w.created_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO workflow_versions (workflow_id, version, doc, author, note, created_at) VALUES (?, 1, ?, ?, ?, ?)",
        )
        .bind(&w.id)
        .bind(body)
        .bind(author.as_str())
        .bind(note)
        .bind(w.created_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Saves a new version (the document's name and description become the workflow's). Returns the version, or
    /// None for an unknown workflow.
    pub async fn save_workflow_version(
        &self,
        id: &str,
        doc: &WorkflowDoc,
        author: Author,
        note: &str,
        now: i64,
    ) -> Result<Option<u32>, StoreError> {
        let body = serde_json::to_string(doc).map_err(|e| corrupt("workflow doc", e))?;
        let mut tx = self.write_tx().await?;
        let Some(next) = next_version(&mut tx, id).await? else {
            return Ok(None);
        };
        sqlx::query(
            "INSERT INTO workflow_versions (workflow_id, version, doc, author, note, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(next)
        .bind(body)
        .bind(author.as_str())
        .bind(note)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE workflows SET version = ?, name = ?, description = ?, updated_at = ? WHERE id = ?")
            .bind(next)
            .bind(&doc.name)
            .bind(&doc.description)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(u32::try_from(next).unwrap_or(0)))
    }

    /// Saves a proposal (Phase 7d spec §6.2): a new version that doesn't become current until accepted. An older
    /// pending proposal is discarded. Returns the version, or None for an unknown workflow.
    pub async fn propose_workflow_version(
        &self,
        id: &str,
        doc: &WorkflowDoc,
        author: Author,
        note: &str,
        now: i64,
    ) -> Result<Option<u32>, StoreError> {
        let body = serde_json::to_string(doc).map_err(|e| corrupt("workflow doc", e))?;
        let mut tx = self.write_tx().await?;
        let Some(next) = next_version(&mut tx, id).await? else {
            return Ok(None);
        };
        sqlx::query("UPDATE workflow_versions SET status = 'discarded' WHERE workflow_id = ? AND status = 'proposed'")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO workflow_versions (workflow_id, version, doc, author, note, created_at, status) \
             VALUES (?, ?, ?, ?, ?, ?, 'proposed')",
        )
        .bind(id)
        .bind(next)
        .bind(body)
        .bind(author.as_str())
        .bind(note)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        // The workflow's `updated_at` stays: a proposal changes nothing that runs until it is accepted, and a schedule
        // takes a change after its last slot as its new baseline (PDT-131).
        tx.commit().await?;
        Ok(Some(u32::try_from(next).unwrap_or(0)))
    }

    /// A version's status (None when either is unknown).
    pub async fn workflow_version_status(&self, id: &str, version: u32) -> Result<Option<VersionStatus>, StoreError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT status FROM workflow_versions WHERE workflow_id = ? AND version = ?")
                .bind(id)
                .bind(i64::from(version))
                .fetch_optional(&self.pool)
                .await?;
        row.map(|(s,)| VersionStatus::parse(&s).ok_or_else(|| corrupt("version status", &s))).transpose()
    }

    /// Makes a proposed version current. False when it isn't a pending proposal.
    pub async fn accept_workflow_version(&self, id: &str, version: u32, now: i64) -> Result<bool, StoreError> {
        let mut tx = self.write_tx().await?;
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT doc FROM workflow_versions WHERE workflow_id = ? AND version = ? AND status = 'proposed'",
        )
        .bind(id)
        .bind(i64::from(version))
        .fetch_optional(&mut *tx)
        .await?;
        let Some((body,)) = row else {
            return Ok(false);
        };
        let doc: WorkflowDoc = serde_json::from_str(&body).map_err(|e| corrupt("workflow doc", e))?;
        sqlx::query("UPDATE workflow_versions SET status = 'saved' WHERE workflow_id = ? AND version = ?")
            .bind(id)
            .bind(i64::from(version))
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE workflows SET version = ?, name = ?, description = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(version))
            .bind(&doc.name)
            .bind(&doc.description)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Sets a proposal aside. False when it isn't a pending proposal.
    pub async fn discard_workflow_version(&self, id: &str, version: u32) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE workflow_versions SET status = 'discarded' WHERE workflow_id = ? AND version = ? \
             AND status = 'proposed'",
        )
        .bind(id)
        .bind(i64::from(version))
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn get_workflow(&self, id: &str) -> Result<Option<Workflow>, StoreError> {
        let sql = format!("SELECT {WF_COLUMNS} FROM workflows WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_workflow).transpose()
    }

    pub async fn workflow_by_builtin(&self, template: &str) -> Result<Option<Workflow>, StoreError> {
        let sql = format!("SELECT {WF_COLUMNS} FROM workflows WHERE builtin = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(template).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_workflow).transpose()
    }

    pub async fn workflow_by_repo_file(&self, project_id: &str, file: &str) -> Result<Option<Workflow>, StoreError> {
        let sql = format!("SELECT {WF_COLUMNS} FROM workflows WHERE project_id = ? AND repo_file = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).bind(file).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_workflow).transpose()
    }

    /// Workflows, by name. With `project`, those that apply to it: its own, host-wide and personal ones.
    pub async fn list_workflows(&self, project: Option<&str>) -> Result<Vec<Workflow>, StoreError> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!("SELECT {WF_COLUMNS} FROM workflows"));
        if let Some(p) = project {
            qb.push(" WHERE project_id IS NULL OR project_id = ").push_bind(p.to_string());
        }
        qb.push(" ORDER BY name COLLATE NOCASE, id");
        let rows = qb.build().fetch_all(&self.pool).await?;
        rows.iter().map(to_workflow).collect()
    }

    pub async fn enabled_workflows(&self) -> Result<Vec<Workflow>, StoreError> {
        let sql = format!("SELECT {WF_COLUMNS} FROM workflows WHERE enabled = 1 ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_workflow).collect()
    }

    pub async fn set_workflow_enabled(&self, id: &str, enabled: bool, now: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE workflows SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(enabled)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_workflow(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM workflows WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// One version's document (None when either is unknown).
    pub async fn workflow_doc(&self, id: &str, version: u32) -> Result<Option<WorkflowDoc>, StoreError> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT doc FROM workflow_versions WHERE workflow_id = ? AND version = ?")
                .bind(id)
                .bind(i64::from(version))
                .fetch_optional(&self.pool)
                .await?;
        row.map(|(d,)| serde_json::from_str(&d).map_err(|e| corrupt("workflow doc", e))).transpose()
    }

    /// Every version, newest first, without documents.
    pub async fn list_workflow_versions(&self, id: &str) -> Result<Vec<WorkflowVersion>, StoreError> {
        let rows: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
            "SELECT version, author, status, note, created_at FROM workflow_versions WHERE workflow_id = ? \
             ORDER BY version DESC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(v, author, status, note, created_at)| {
                Ok(WorkflowVersion {
                    workflow_id: id.into(),
                    version: u32::try_from(v).unwrap_or(0),
                    author: Author::parse(&author).ok_or_else(|| corrupt("version author", &author))?,
                    status: VersionStatus::parse(&status).ok_or_else(|| corrupt("version status", &status))?,
                    note,
                    created_at,
                    doc: None,
                })
            })
            .collect()
    }

    /// Who wrote the current version.
    pub async fn current_version_author(&self, id: &str) -> Result<Option<Author>, StoreError> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT v.author FROM workflow_versions v JOIN workflows w ON w.id = v.workflow_id \
             WHERE w.id = ? AND v.version = w.version",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(a,)| Author::parse(&a)))
    }

    // ---- runs and steps ----

    /// The workflow's run with `key`.
    pub async fn workflow_run_by_key(&self, workflow_id: &str, key: &str) -> Result<Option<Run>, StoreError> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id \
             WHERE r.workflow_id = ? AND r.run_key = ?"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(workflow_id).bind(key).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_run).transpose()
    }

    /// Starts a run with its first steps (the trigger's, and the ones it leads to), unless the workflow has a run
    /// with its key: false then.
    /// How many events of `kind` whose payload's `field` is `value` started a workflow run since `since`, leaving out
    /// the event `except` (PDT-202: the triage caps count issues per author and per repository).
    pub async fn events_with_runs_since(
        &self,
        kind: &str,
        field: &str,
        value: &str,
        since: i64,
        except: &str,
    ) -> Result<u32, StoreError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(DISTINCT trigger_event_id) FROM workflow_runs WHERE started_at >= ? \
             AND trigger_event_id IS NOT NULL AND trigger_event_id != ? \
             AND json_extract(trigger, '$.type') = ? AND json_extract(trigger, '$.' || ?) = ?",
        )
        .bind(since)
        .bind(except)
        .bind(kind)
        .bind(field)
        .bind(value)
        .fetch_one(&self.pool)
        .await?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    pub async fn start_workflow_run(&self, run: &Run, steps: &[Step]) -> Result<bool, StoreError> {
        let mut tx = self.write_tx().await?;
        let r = sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_id, version, status, run_key, trigger_event_id, trigger_node, \
             trigger, project_id, ticket_id, attempt_id, parent_run_id, parent_step_id, error, output, wake_at, \
             started_at, ended_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (workflow_id, run_key) DO NOTHING",
        )
        .bind(&run.id)
        .bind(&run.workflow_id)
        .bind(i64::from(run.version))
        .bind(run.status.as_str())
        .bind(&run.run_key)
        .bind(&run.trigger_event_id)
        .bind(&run.trigger_node)
        .bind(self.redactor.string(&run.trigger.to_string()))
        .bind(&run.project_id)
        .bind(&run.ticket_id)
        .bind(&run.attempt_id)
        .bind(&run.parent_run_id)
        .bind(&run.parent_step_id)
        .bind(&run.error)
        .bind(opt_json(&run.output))
        .bind(run.wake_at)
        .bind(run.started_at)
        .bind(run.ended_at)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 0 {
            return Ok(false);
        }
        for s in steps {
            put_step(&mut tx, &self.redactor, s).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn get_workflow_run(&self, id: &str) -> Result<Option<Run>, StoreError> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id WHERE r.id = ?"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_run).transpose()
    }

    /// Runs, newest first.
    pub async fn list_workflow_runs(&self, f: &RunFilter) -> Result<Vec<Run>, StoreError> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {RUN_COLUMNS} FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id WHERE 1 = 1"
        ));
        if let Some(w) = &f.workflow_id {
            qb.push(" AND r.workflow_id = ").push_bind(w.clone());
        }
        if let Some(t) = &f.ticket_id {
            qb.push(" AND r.ticket_id = ").push_bind(t.clone());
        }
        if let Some(s) = f.status {
            qb.push(" AND r.status = ").push_bind(s.as_str());
        }
        qb.push(" ORDER BY r.started_at DESC, r.id DESC LIMIT ").push_bind(f.limit.clamp(1, 500));
        let rows = qb.build().fetch_all(&self.pool).await?;
        rows.iter().map(to_run).collect()
    }

    /// Runs the engine should look at now: running ones, and waiting ones whose wake time has come.
    pub async fn runnable_workflow_runs(&self, now: i64) -> Result<Vec<Run>, StoreError> {
        let sql = format!(
            "SELECT {RUN_COLUMNS} FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id \
             WHERE r.status = 'running' OR (r.status = 'waiting' AND r.wake_at IS NOT NULL AND r.wake_at <= ?) \
             ORDER BY r.started_at, r.id"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(now).fetch_all(&self.pool).await?;
        rows.iter().map(to_run).collect()
    }

    /// Writes a run's changing fields: status, error, output, wake time, when it was resumed, end.
    pub async fn update_workflow_run(&self, run: &Run) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE workflow_runs SET status = ?, error = ?, output = ?, wake_at = ?, resumed_at = ?, ended_at = ? \
             WHERE id = ?",
        )
        .bind(run.status.as_str())
        .bind(run.error.as_deref().map(|e| self.redactor.string(e)))
        .bind(opt_json(&self.redactor.json(&run.output)))
        .bind(run.wake_at)
        .bind(run.resumed_at)
        .bind(run.ended_at)
        .bind(&run.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Marks a waiting run to be looked at now (a step it waits on was resolved).
    pub async fn wake_workflow_run(&self, id: &str, now: i64) -> Result<(), StoreError> {
        sqlx::query("UPDATE workflow_runs SET status = 'running', wake_at = NULL WHERE id = ? AND status = 'waiting'")
            .bind(id)
            .execute(&self.pool)
            .await?;
        let _ = now;
        Ok(())
    }

    /// Waiting runs with no wake time (left by a PDT before PDT-131, which set none for a wait with no timeout) wake
    /// now, so the engine looks at them once: it gives them their runtime limit as a wake time, and goes on with one a
    /// crash left with nothing to wait for. Returns how many woke.
    pub async fn wake_untimed_workflow_runs(&self, now: i64) -> Result<u64, StoreError> {
        let r = sqlx::query("UPDATE workflow_runs SET wake_at = ? WHERE status = 'waiting' AND wake_at IS NULL")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected())
    }

    /// Ends a step and queues what it leads to in one transaction (PDT-131), so a crash never leaves a run with the
    /// step done and nothing after it: the step's open Inbox items close, its agent binding is done, and its run, if
    /// waiting, wakes.
    pub async fn complete_workflow_step(&self, step: &Step, next: &[Step], now: i64) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        put_step(&mut tx, &self.redactor, step).await?;
        for n in next {
            put_step(&mut tx, &self.redactor, n).await?;
        }
        sqlx::query("UPDATE inbox_items SET status = 'closed', resolved_at = ? WHERE status = 'open' AND step_id = ?")
            .bind(now)
            .bind(&step.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE agent_steps SET state = 'done', updated_at = ? WHERE step_id = ? AND state != 'done'")
            .bind(now)
            .bind(&step.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE workflow_runs SET status = 'running', wake_at = NULL WHERE id = ? AND status = 'waiting'")
            .bind(&step.run_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records what a running step has done so far (its `output`, e.g. the messages it posted), so running it
    /// again after a restart or a retry doesn't do it twice. False when the step is no longer at that execution.
    pub async fn set_step_progress(&self, id: &str, attempts: u32, output: &Value) -> Result<bool, StoreError> {
        let r =
            sqlx::query("UPDATE workflow_steps SET output = ? WHERE id = ? AND status = 'running' AND attempts = ?")
                .bind(opt_json(&self.redactor.json(output)))
                .bind(id)
                .bind(i64::from(attempts))
                .execute(&self.pool)
                .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Closes the open Inbox items of every run of a workflow (it is being deleted: nothing could answer them).
    pub async fn close_workflow_inbox_items(&self, workflow_id: &str, now: i64) -> Result<u64, StoreError> {
        let r = sqlx::query(
            "UPDATE inbox_items SET status = 'closed', resolved_at = ? WHERE status = 'open' AND \
             run_id IN (SELECT id FROM workflow_runs WHERE workflow_id = ?)",
        )
        .bind(now)
        .bind(workflow_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Writes a step that is to run now, and wakes its run if it waits, in one transaction.
    pub async fn put_step_and_wake(&self, s: &Step) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        put_step(&mut tx, &self.redactor, s).await?;
        sqlx::query("UPDATE workflow_runs SET status = 'running', wake_at = NULL WHERE id = ? AND status = 'waiting'")
            .bind(&s.run_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Adds or replaces a step.
    pub async fn put_step(&self, s: &Step) -> Result<(), StoreError> {
        let mut conn = self.pool.acquire().await?;
        put_step(&mut conn, &self.redactor, s).await
    }

    pub async fn get_step(&self, id: &str) -> Result<Option<Step>, StoreError> {
        let sql = format!("SELECT {STEP_COLUMNS} FROM workflow_steps WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_step).transpose()
    }

    /// A run's steps in order.
    pub async fn list_steps(&self, run_id: &str) -> Result<Vec<Step>, StoreError> {
        let sql = format!("SELECT {STEP_COLUMNS} FROM workflow_steps WHERE run_id = ? ORDER BY seq");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(run_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_step).collect()
    }

    /// Steps waiting for an event, with their runs' workflow ids.
    pub async fn steps_waiting_for_events(&self) -> Result<Vec<(String, Step)>, StoreError> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT r.workflow_id AS wf, {} FROM workflow_steps s JOIN workflow_runs r ON r.id = s.run_id \
             WHERE s.status = 'waiting' AND json_extract(s.wait, '$.event') IS NOT NULL ORDER BY s.seq",
            STEP_COLUMNS.split(", ").map(|c| format!("s.{c}")).collect::<Vec<_>>().join(", ")
        )))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok((r.try_get("wf")?, to_step(r)?))).collect()
    }

    /// Steps waiting for a slot under the host's cap on agents running a turn (Phase 7c §6.2), oldest first, with
    /// their runs' tickets.
    pub async fn steps_waiting_for_slot(&self) -> Result<Vec<(Option<String>, Step)>, StoreError> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT r.ticket_id AS tk, {} FROM workflow_steps s JOIN workflow_runs r ON r.id = s.run_id \
             WHERE s.status = 'waiting' AND json_extract(s.wait, '$.slot') IS NOT NULL ORDER BY s.started_at, s.id",
            STEP_COLUMNS.split(", ").map(|c| format!("s.{c}")).collect::<Vec<_>>().join(", ")
        )))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok((r.try_get("tk")?, to_step(r)?))).collect()
    }

    /// Deletes finished runs that ended before `before`. Returns how many went.
    pub async fn prune_workflow_runs(&self, before: i64) -> Result<u64, StoreError> {
        let r = sqlx::query(
            "DELETE FROM workflow_runs WHERE ended_at IS NOT NULL AND ended_at < ? \
             AND status IN ('succeeded', 'quiet', 'failed', 'cancelled')",
        )
        .bind(before)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    // ---- workflow state ----

    pub async fn workflow_state(&self, workflow_id: &str) -> Result<serde_json::Map<String, Value>, StoreError> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM workflow_state WHERE workflow_id = ? ORDER BY key")
                .bind(workflow_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(k, v)| (k, serde_json::from_str(&v).unwrap_or(Value::Null))).collect())
    }

    /// Sets a state value; with `if_missing`, only when the key has none. True when it was written.
    pub async fn set_workflow_state(
        &self,
        workflow_id: &str,
        key: &str,
        value: &Value,
        if_missing: bool,
        now: i64,
    ) -> Result<bool, StoreError> {
        let sql = if if_missing {
            "INSERT INTO workflow_state (workflow_id, key, value, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (workflow_id, key) DO NOTHING"
        } else {
            "INSERT INTO workflow_state (workflow_id, key, value, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (workflow_id, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at"
        };
        let r =
            sqlx::query(sql).bind(workflow_id).bind(key).bind(value.to_string()).bind(now).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Adds `by` to a number in the state (none, or not a number, counts as 0) and returns the new value.
    pub async fn add_workflow_state(&self, workflow_id: &str, key: &str, by: f64, now: i64) -> Result<f64, StoreError> {
        let mut tx = self.write_tx().await?;
        let old: Option<String> =
            sqlx::query_scalar("SELECT value FROM workflow_state WHERE workflow_id = ? AND key = ?")
                .bind(workflow_id)
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?;
        let old = old.and_then(|v| serde_json::from_str::<Value>(&v).ok()).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let new = old + by;
        let value = serde_json::Number::from_f64(new).map(Value::Number).unwrap_or(Value::Null);
        // Whole numbers stay integers (`3`, not `3.0`).
        let value = match value.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 9e15 => Value::from(f as i64),
            _ => value,
        };
        sqlx::query(
            "INSERT INTO workflow_state (workflow_id, key, value, updated_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT (workflow_id, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(workflow_id)
        .bind(key)
        .bind(value.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(new)
    }

    // ---- session outbox ----

    /// Queues a message; a step's message is queued once (a re-run step finds its own).
    pub async fn queue_session_message(&self, m: &SessionMessage) -> Result<SessionMessage, StoreError> {
        let mut tx = self.write_tx().await?;
        if let Some(step) = &m.step_id {
            let sql = format!("SELECT {MSG_COLUMNS} FROM session_messages WHERE step_id = ?");
            if let Some(r) = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(step).fetch_optional(&mut *tx).await? {
                return to_message(&r);
            }
        }
        sqlx::query(
            "INSERT INTO session_messages (id, attempt_id, session_id, ticket_id, dedupe, text, timeline, run_id, \
             step_id, status, told_session, created_at, resolved_at, by_user, pending_id, reopens) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&m.id)
        .bind(&m.attempt_id)
        .bind(&m.session_id)
        .bind(&m.ticket_id)
        .bind(&m.dedupe)
        .bind(self.redactor.string(&m.text))
        .bind(opt_json(&m.timeline))
        .bind(&m.run_id)
        .bind(&m.step_id)
        .bind(&m.status)
        .bind(&m.told_session)
        .bind(m.created_at)
        .bind(m.resolved_at)
        .bind(m.by_user)
        .bind(&m.pending_id)
        .bind(m.reopens)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(m.clone())
    }

    /// Whether a message with `dedupe` for the attempt is waiting, on its way or told. One nobody was told does not
    /// count: a later notice with its key may still be told (PDT-129).
    pub async fn session_message_sent(&self, attempt_id: &str, dedupe: &str) -> Result<bool, StoreError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM session_messages WHERE attempt_id = ? AND dedupe = ? AND status != 'untold'",
        )
        .bind(attempt_id)
        .bind(dedupe)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Messages waiting for their session, oldest first.
    pub async fn waiting_session_messages(&self) -> Result<Vec<SessionMessage>, StoreError> {
        let sql =
            format!("SELECT {MSG_COLUMNS} FROM session_messages WHERE status = 'waiting' ORDER BY created_at, id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_message).collect()
    }

    /// The newest message to `session_id` with `dedupe` (an agent step's first message to its session, PDT-144).
    pub async fn latest_session_message(
        &self,
        session_id: &str,
        dedupe: &str,
    ) -> Result<Option<SessionMessage>, StoreError> {
        let sql = format!(
            "SELECT {MSG_COLUMNS} FROM session_messages WHERE session_id = ? AND dedupe = ? \
             ORDER BY created_at DESC, id DESC LIMIT 1"
        );
        let row =
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(session_id).bind(dedupe).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_message).transpose()
    }

    pub async fn get_session_message(&self, id: &str) -> Result<Option<SessionMessage>, StoreError> {
        let sql = format!("SELECT {MSG_COLUMNS} FROM session_messages WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_message).transpose()
    }

    /// Messages a conversation has and has not echoed yet: those of one pending message, or all of them.
    pub async fn sending_session_messages(&self, pending_id: Option<&str>) -> Result<Vec<SessionMessage>, StoreError> {
        let sql = format!(
            "SELECT {MSG_COLUMNS} FROM session_messages WHERE status = 'sending' AND (? IS NULL OR pending_id = ?) \
             ORDER BY created_at, id"
        );
        let rows =
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(pending_id).bind(pending_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_message).collect()
    }

    /// Hands waiting messages to a conversation's pending message `pending_id` (on their way to `session`). Returns
    /// how many it took.
    pub async fn send_session_messages(
        &self,
        ids: &[String],
        session: &str,
        pending_id: &str,
    ) -> Result<u64, StoreError> {
        let mut tx = self.write_tx().await?;
        let mut n = 0;
        for id in ids {
            n += sqlx::query(
                "UPDATE session_messages SET status = 'sending', told_session = ?, pending_id = ? \
                 WHERE id = ? AND status = 'waiting'",
            )
            .bind(session)
            .bind(pending_id)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        }
        tx.commit().await?;
        Ok(n)
    }

    /// Puts the messages of a pending message that never went out back to waiting.
    pub async fn unsend_session_messages(&self, pending_id: &str) -> Result<u64, StoreError> {
        let r = sqlx::query(
            "UPDATE session_messages SET status = 'waiting', told_session = NULL, pending_id = NULL \
             WHERE pending_id = ? AND status = 'sending'",
        )
        .bind(pending_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    /// Resolves a waiting (or sending) message: told to `session`, or to nobody. False when it was already resolved.
    pub async fn resolve_session_message(&self, id: &str, session: Option<&str>, now: i64) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE session_messages SET status = ?, told_session = ?, resolved_at = ? \
             WHERE id = ? AND status IN ('waiting', 'sending')",
        )
        .bind(if session.is_some() { "told" } else { "untold" })
        .bind(session)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Deletes messages resolved before `before`, except those of an active attempt (its `dedupe` keys still count).
    /// Returns how many went.
    pub async fn prune_session_messages(&self, before: i64) -> Result<u64, StoreError> {
        let r = sqlx::query(
            "DELETE FROM session_messages WHERE status != 'waiting' AND resolved_at < ? AND (attempt_id IS NULL OR \
             attempt_id NOT IN (SELECT id FROM attempts WHERE state = 'active'))",
        )
        .bind(before)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    // ---- templates ----

    pub async fn insert_template(&self, t: &Template) -> Result<(), StoreError> {
        let params = serde_json::to_string(&t.params).map_err(|e| corrupt("template params", e))?;
        let doc = serde_json::to_string(&t.doc).map_err(|e| corrupt("template doc", e))?;
        sqlx::query(
            "INSERT INTO workflow_templates (id, name, description, params, doc, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&t.id)
        .bind(&t.name)
        .bind(&t.description)
        .bind(params)
        .bind(doc)
        .bind(t.created_at)
        .bind(t.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_template(&self, t: &Template) -> Result<bool, StoreError> {
        let params = serde_json::to_string(&t.params).map_err(|e| corrupt("template params", e))?;
        let doc = serde_json::to_string(&t.doc).map_err(|e| corrupt("template doc", e))?;
        let r = sqlx::query(
            "UPDATE workflow_templates SET name = ?, description = ?, params = ?, doc = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&t.name)
        .bind(&t.description)
        .bind(params)
        .bind(doc)
        .bind(t.updated_at)
        .bind(&t.id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn get_template(&self, id: &str) -> Result<Option<Template>, StoreError> {
        let row =
            sqlx::query("SELECT * FROM workflow_templates WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_template).transpose()
    }

    /// The user's templates, by name.
    pub async fn list_templates(&self) -> Result<Vec<Template>, StoreError> {
        let rows = sqlx::query("SELECT * FROM workflow_templates ORDER BY name COLLATE NOCASE, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(to_template).collect()
    }

    pub async fn delete_template(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM workflow_templates WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    // ---- connections ----

    pub async fn insert_connection(&self, c: &Connection) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO connections (id, name, kind, url, config, has_secret, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.name)
        .bind(&c.kind)
        .bind(&c.url)
        .bind(c.config.to_string())
        .bind(c.has_secret)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Writes a connection's name, kind, URL, config and secret flag, and forgets its last error (the next use tells
    /// whether the change fixed it).
    pub async fn update_connection(&self, c: &Connection) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE connections SET name = ?, kind = ?, url = ?, config = ?, has_secret = ?, updated_at = ?, \
             last_error = NULL, last_error_at = NULL WHERE id = ?",
        )
        .bind(&c.name)
        .bind(&c.kind)
        .bind(&c.url)
        .bind(c.config.to_string())
        .bind(c.has_secret)
        .bind(c.updated_at)
        .bind(&c.id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn get_connection(&self, id: &str) -> Result<Option<Connection>, StoreError> {
        let sql = format!("SELECT {CONN_COLUMNS} FROM connections WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_connection).transpose()
    }

    pub async fn connection_by_name(&self, name: &str) -> Result<Option<Connection>, StoreError> {
        let sql = format!("SELECT {CONN_COLUMNS} FROM connections WHERE name = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(name).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_connection).transpose()
    }

    pub async fn list_connections(&self) -> Result<Vec<Connection>, StoreError> {
        let sql = format!("SELECT {CONN_COLUMNS} FROM connections ORDER BY name COLLATE NOCASE");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_connection).collect()
    }

    /// Records how a sensor's use of a connection went: `error` on failure, None once it works. True when that changed
    /// whether it is failing, or why (so the caller notifies, or clears, once).
    pub async fn set_connection_error(&self, id: &str, error: Option<&str>, now: i64) -> Result<bool, StoreError> {
        let r = match error {
            Some(e) => {
                sqlx::query(
                    "UPDATE connections SET last_error = ?, last_error_at = ? \
                     WHERE id = ? AND last_error IS NOT ?",
                )
                .bind(e)
                .bind(now)
                .bind(id)
                .bind(e)
                .execute(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "UPDATE connections SET last_error = NULL, last_error_at = NULL \
                     WHERE id = ? AND last_error IS NOT NULL",
                )
                .bind(id)
                .execute(&self.pool)
                .await?
            }
        };
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_connection(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM connections WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }
}

/// Known secrets' values are redacted from what a step took and made (PDT-158).
async fn put_step(conn: &mut sqlx::SqliteConnection, r: &Redactor, s: &Step) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO workflow_steps (id, run_id, seq, node_id, kind, status, input, output, port, attempts, error, \
         wake_at, wait, started_at, ended_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET status = excluded.status, input = excluded.input, output = excluded.output, \
         port = excluded.port, attempts = excluded.attempts, error = excluded.error, wake_at = excluded.wake_at, \
         wait = excluded.wait, started_at = excluded.started_at, ended_at = excluded.ended_at",
    )
    .bind(&s.id)
    .bind(&s.run_id)
    .bind(s.seq)
    .bind(&s.node_id)
    .bind(&s.kind)
    .bind(s.status.as_str())
    .bind(opt_json(&r.json(&s.input)))
    .bind(opt_json(&r.json(&s.output)))
    .bind(&s.port)
    .bind(i64::from(s.attempts))
    .bind(s.error.as_deref().map(|e| r.string(e)))
    .bind(s.wake_at)
    .bind(opt_json(&r.json(&s.wait)))
    .bind(s.started_at)
    .bind(s.ended_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;
    use serde_json::json;

    fn doc(name: &str) -> WorkflowDoc {
        serde_json::from_value(json!({"name": name, "nodes": [{"id": "t", "kind": "manual", "version": 1}]})).unwrap()
    }

    fn wf(id: &str, scope: Scope, project: Option<&str>) -> Workflow {
        Workflow {
            id: id.into(),
            name: String::new(),
            description: String::new(),
            scope,
            project_id: project.map(String::from),
            enabled: true,
            builtin: None,
            source: Source::Host,
            repo_file: None,
            version: 1,
            proposed: None,
            created_at: 5,
            updated_at: 5,
        }
    }

    fn run(id: &str, key: &str) -> Run {
        Run {
            id: id.into(),
            workflow_id: "w".into(),
            workflow_name: String::new(),
            version: 1,
            status: RunStatus::Running,
            run_key: key.into(),
            trigger_event_id: None,
            trigger_node: "t".into(),
            trigger: json!({"x": 1}),
            project_id: Some("p".into()),
            ticket_id: Some("tk".into()),
            attempt_id: None,
            parent_run_id: None,
            parent_step_id: None,
            error: None,
            output: Value::Null,
            wake_at: None,
            started_at: 10,
            resumed_at: None,
            ended_at: None,
        }
    }

    fn step(id: &str, run: &str, seq: i64) -> Step {
        Step {
            id: id.into(),
            run_id: run.into(),
            seq,
            node_id: "t".into(),
            kind: "manual".into(),
            status: StepStatus::Succeeded,
            input: Value::Null,
            output: json!({"x": 1}),
            port: Some("out".into()),
            attempts: 1,
            error: None,
            wake_at: None,
            wait: Value::Null,
            started_at: Some(10),
            ended_at: Some(10),
        }
    }

    #[tokio::test]
    async fn workflows_keep_every_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        store.insert_workflow(&wf("w", Scope::Project, Some("p")), &doc("one"), Author::Builtin, "").await.unwrap();
        store.insert_workflow(&wf("h", Scope::Host, None), &doc("host"), Author::User, "").await.unwrap();
        assert_eq!(store.save_workflow_version("w", &doc("two"), Author::User, "renamed", 9).await.unwrap(), Some(2));
        assert_eq!(store.save_workflow_version("nope", &doc("x"), Author::User, "", 9).await.unwrap(), None);
        let w = store.get_workflow("w").await.unwrap().unwrap();
        assert_eq!((w.name.as_str(), w.version, w.updated_at), ("two", 2, 9));
        assert_eq!(store.workflow_doc("w", 1).await.unwrap().unwrap().name, "one");
        let versions = store.list_workflow_versions("w").await.unwrap();
        assert_eq!(
            versions.iter().map(|v| (v.version, v.author)).collect::<Vec<_>>(),
            [(2, Author::User), (1, Author::Builtin)]
        );
        assert_eq!(store.current_version_author("w").await.unwrap(), Some(Author::User));
        assert_eq!(store.list_workflows(Some("p")).await.unwrap().len(), 2);
        assert_eq!(store.list_workflows(Some("other")).await.unwrap().len(), 1, "only the host-wide one");
        store.set_workflow_enabled("h", false, 11).await.unwrap();
        assert_eq!(store.enabled_workflows().await.unwrap().len(), 1);
        assert!(store.delete_workflow("h").await.unwrap());
    }

    #[tokio::test]
    async fn proposals_wait_for_review() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        store.insert_workflow(&wf("w", Scope::Host, None), &doc("one"), Author::User, "").await.unwrap();
        assert_eq!(store.propose_workflow_version("w", &doc("agent"), Author::Agent, "a", 7).await.unwrap(), Some(2));
        let w = store.get_workflow("w").await.unwrap().unwrap();
        assert_eq!((w.name.as_str(), w.version, w.proposed), ("one", 1, Some(2)), "the current version stays");
        assert_eq!(w.updated_at, 5, "a proposal is no change to what runs (a schedule's baseline stays)");
        // A save meanwhile takes the next number; a newer proposal sets the older one aside.
        assert_eq!(store.save_workflow_version("w", &doc("mine"), Author::User, "", 8).await.unwrap(), Some(3));
        assert_eq!(store.propose_workflow_version("w", &doc("agent 2"), Author::Agent, "", 9).await.unwrap(), Some(4));
        assert_eq!(store.workflow_version_status("w", 2).await.unwrap(), Some(VersionStatus::Discarded));
        assert!(!store.accept_workflow_version("w", 2, 10).await.unwrap(), "a discarded one can't be accepted");
        assert!(store.accept_workflow_version("w", 4, 10).await.unwrap());
        let w = store.get_workflow("w").await.unwrap().unwrap();
        assert_eq!((w.name.as_str(), w.version, w.proposed), ("agent 2", 4, None));
        let statuses: Vec<_> =
            store.list_workflow_versions("w").await.unwrap().iter().map(|v| (v.version, v.status)).collect();
        assert_eq!(
            statuses,
            [
                (4, VersionStatus::Saved),
                (3, VersionStatus::Saved),
                (2, VersionStatus::Discarded),
                (1, VersionStatus::Saved)
            ]
        );
        assert_eq!(store.propose_workflow_version("w", &doc("x"), Author::Agent, "", 11).await.unwrap(), Some(5));
        assert!(store.discard_workflow_version("w", 5).await.unwrap());
        assert!(!store.discard_workflow_version("w", 5).await.unwrap());
        assert_eq!(store.get_workflow("w").await.unwrap().unwrap().proposed, None);
        assert_eq!(store.propose_workflow_version("nope", &doc("x"), Author::Agent, "", 11).await.unwrap(), None);
    }

    #[tokio::test]
    async fn templates_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let mut t = Template {
            id: "t1".into(),
            name: "Zed".into(),
            description: "d".into(),
            builtin: false,
            category: Default::default(),
            params: serde_json::from_value(json!([{"id": "x", "title": "X", "type": "string"}])).unwrap(),
            doc: doc("{{params.x}}"),
            created_at: 1,
            updated_at: 1,
        };
        store.insert_template(&t).await.unwrap();
        store.insert_template(&Template { id: "t2".into(), name: "alpha".into(), ..t.clone() }).await.unwrap();
        t.name = "Zeta".into();
        t.updated_at = 2;
        assert!(store.update_template(&t).await.unwrap());
        assert_eq!(store.get_template("t1").await.unwrap().unwrap(), t);
        let names: Vec<_> = store.list_templates().await.unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["alpha", "Zeta"]);
        assert!(store.delete_template("t2").await.unwrap());
        assert!(store.get_template("t2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_run_key_starts_one_run_and_steps_are_upserted() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        store.insert_workflow(&wf("w", Scope::Host, None), &doc("w"), Author::User, "").await.unwrap();
        assert!(store.start_workflow_run(&run("r1", "k"), &[step("s1", "r1", 1)]).await.unwrap());
        assert!(!store.start_workflow_run(&run("r2", "k"), &[step("s2", "r2", 1)]).await.unwrap(), "same key: skipped");
        assert!(store.get_workflow_run("r2").await.unwrap().is_none());
        let mut s = step("s3", "r1", 2);
        s.status = StepStatus::Waiting;
        s.wait = json!({"event": {"type": "pr.merged"}});
        store.put_step(&s).await.unwrap();
        s.attempts = 2;
        store.put_step(&s).await.unwrap();
        let steps = store.list_steps("r1").await.unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].attempts, 2);
        assert_eq!(store.steps_waiting_for_events().await.unwrap()[0].0, "w");

        let mut r = store.get_workflow_run("r1").await.unwrap().unwrap();
        assert_eq!(r.trigger, json!({"x": 1}));
        assert_eq!(store.runnable_workflow_runs(0).await.unwrap().len(), 1);
        r.status = RunStatus::Waiting;
        r.wake_at = Some(100);
        store.update_workflow_run(&r).await.unwrap();
        assert!(store.runnable_workflow_runs(50).await.unwrap().is_empty());
        assert_eq!(store.runnable_workflow_runs(100).await.unwrap().len(), 1);
        r.wake_at = None;
        store.update_workflow_run(&r).await.unwrap();
        store.wake_workflow_run("r1", 1).await.unwrap();
        assert_eq!(store.get_workflow_run("r1").await.unwrap().unwrap().status, RunStatus::Running);
        let f = RunFilter { ticket_id: Some("tk".into()), limit: 10, ..Default::default() };
        assert_eq!(store.list_workflow_runs(&f).await.unwrap()[0].workflow_name, "w");
        r.status = RunStatus::Succeeded;
        r.ended_at = Some(20);
        store.update_workflow_run(&r).await.unwrap();
        assert_eq!(store.prune_workflow_runs(21).await.unwrap(), 1);
        assert!(store.list_steps("r1").await.unwrap().is_empty(), "steps go with their run");
    }

    #[tokio::test]
    async fn progress_is_kept_for_the_execution_that_made_it_and_completion_is_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        store.insert_workflow(&wf("w", Scope::Host, None), &doc("w"), Author::User, "").await.unwrap();
        let mut gate = step("g", "r1", 2);
        gate.status = StepStatus::Waiting;
        assert!(store.start_workflow_run(&run("r1", "k"), &[step("s1", "r1", 1), gate.clone()]).await.unwrap());
        let mut r = store.get_workflow_run("r1").await.unwrap().unwrap();
        r.status = RunStatus::Waiting;
        store.update_workflow_run(&r).await.unwrap();
        assert_eq!(store.wake_untimed_workflow_runs(7).await.unwrap(), 1, "no wake time: woken");
        assert_eq!(store.get_workflow_run("r1").await.unwrap().unwrap().wake_at, Some(7));

        let mut post = step("p", "r1", 3);
        post.status = StepStatus::Running;
        post.output = Value::Null;
        store.put_step(&post).await.unwrap();
        assert!(store.set_step_progress("p", 1, &json!({"messageIds": ["m1"]})).await.unwrap());
        assert!(!store.set_step_progress("p", 2, &json!({})).await.unwrap(), "another execution's");
        assert_eq!(store.get_step("p").await.unwrap().unwrap().output, json!({"messageIds": ["m1"]}));

        gate.status = StepStatus::Succeeded;
        store.complete_workflow_step(&gate, &[step("n", "r1", 4)], 9).await.unwrap();
        let steps = store.list_steps("r1").await.unwrap();
        assert_eq!(steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s1", "g", "p", "n"]);
        assert_eq!(store.get_workflow_run("r1").await.unwrap().unwrap().status, RunStatus::Running, "woken");
    }

    #[tokio::test]
    async fn state_outbox_and_connections() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        store.insert_workflow(&wf("w", Scope::Host, None), &doc("w"), Author::User, "").await.unwrap();
        assert!(store.set_workflow_state("w", "seen", &json!([1]), true, 1).await.unwrap());
        assert!(!store.set_workflow_state("w", "seen", &json!([2]), true, 2).await.unwrap(), "baseline kept");
        store.set_workflow_state("w", "seen", &json!([3]), false, 3).await.unwrap();
        assert_eq!(store.workflow_state("w").await.unwrap()["seen"], json!([3]));
        // A counter (PDT-104): from nothing, and a non-number counts as 0.
        assert_eq!(store.add_workflow_state("w", "n", 1.0, 4).await.unwrap(), 1.0);
        assert_eq!(store.add_workflow_state("w", "n", 2.0, 5).await.unwrap(), 3.0);
        assert_eq!(store.workflow_state("w").await.unwrap()["n"], json!(3));
        assert_eq!(store.add_workflow_state("w", "seen", 1.0, 6).await.unwrap(), 1.0);

        let m = SessionMessage {
            id: "m".into(),
            attempt_id: Some("a".into()),
            session_id: None,
            ticket_id: Some("t".into()),
            dedupe: Some("conflict:h".into()),
            text: "hi".into(),
            timeline: json!({"kind": "pr"}),
            run_id: None,
            step_id: Some("s".into()),
            status: "waiting".into(),
            told_session: None,
            created_at: 1,
            resolved_at: None,
            by_user: true,
            pending_id: None,
            reopens: true,
        };
        store.queue_session_message(&m).await.unwrap();
        assert!(store.waiting_session_messages().await.unwrap()[0].by_user);
        assert!(store.waiting_session_messages().await.unwrap()[0].reopens);
        let again = SessionMessage { id: "m2".into(), ..m.clone() };
        assert_eq!(store.queue_session_message(&again).await.unwrap().id, "m", "one per step");
        assert!(store.session_message_sent("a", "conflict:h").await.unwrap());
        assert_eq!(store.waiting_session_messages().await.unwrap().len(), 1);
        assert!(store.resolve_session_message("m", Some("sess"), 5).await.unwrap());
        assert!(!store.resolve_session_message("m", None, 6).await.unwrap());
        let got = store.get_session_message("m").await.unwrap().unwrap();
        assert_eq!((got.status.as_str(), got.told_session.as_deref()), ("told", Some("sess")));
        let waiting = SessionMessage { id: "w".into(), step_id: Some("s2".into()), ..m.clone() };
        store.queue_session_message(&waiting).await.unwrap();
        // An active attempt's messages stay: their dedupe keys still keep a notice from being sent twice.
        for sql in [
            "INSERT INTO tickets (id, seq, project_id, number, title, stage, priority, rank, created_at, updated_at) \
             VALUES ('tk', 1, 'p', 1, 't', 'in_progress', 'medium', 'a', 1, 1)",
            "INSERT INTO attempts (id, ticket_id, branch, base_branch, base_commit, state, created_at) \
             VALUES ('live', 'tk', 'b', 'main', 'abc', 'active', 1)",
        ] {
            sqlx::query(sql).execute(&store.pool).await.unwrap();
        }
        let live = SessionMessage {
            id: "l".into(),
            attempt_id: Some("live".into()),
            step_id: Some("s3".into()),
            status: "told".into(),
            resolved_at: Some(5),
            ..m.clone()
        };
        store.queue_session_message(&live).await.unwrap();
        assert_eq!(store.prune_session_messages(5).await.unwrap(), 0, "resolved at `before`: kept");
        assert_eq!(store.prune_session_messages(100).await.unwrap(), 1, "the told one goes, the waiting one stays");
        assert!(store.get_session_message("m").await.unwrap().is_none());
        assert!(store.get_session_message("w").await.unwrap().is_some());
        assert!(store.get_session_message("l").await.unwrap().is_some(), "the active attempt's stays");

        // On its way to a conversation (PDT-129): sending until the echo, back to waiting when it never went out.
        assert_eq!(store.send_session_messages(&["w".into()], "sess", "p1").await.unwrap(), 1);
        assert!(store.waiting_session_messages().await.unwrap().is_empty());
        assert_eq!(store.sending_session_messages(Some("p1")).await.unwrap().len(), 1);
        assert!(store.sending_session_messages(Some("p2")).await.unwrap().is_empty());
        assert_eq!(store.unsend_session_messages("p1").await.unwrap(), 1);
        assert_eq!(store.waiting_session_messages().await.unwrap()[0].told_session, None);
        store.send_session_messages(&["w".into()], "sess", "p1").await.unwrap();
        assert_eq!(store.sending_session_messages(None).await.unwrap()[0].pending_id.as_deref(), Some("p1"));
        assert!(store.resolve_session_message("w", None, 7).await.unwrap(), "a sending one resolves");
        // Nobody was told: its dedupe key is not spent.
        let untold =
            SessionMessage { id: "u".into(), step_id: Some("s4".into()), dedupe: Some("k".into()), ..m.clone() };
        store.queue_session_message(&untold).await.unwrap();
        assert!(store.session_message_sent("a", "k").await.unwrap(), "waiting counts");
        store.resolve_session_message("u", None, 8).await.unwrap();
        assert!(!store.session_message_sent("a", "k").await.unwrap(), "untold does not");
        // A step's first message to a session (PDT-144): the newest with its key.
        let first = SessionMessage {
            id: "f1".into(),
            session_id: Some("sx".into()),
            step_id: None,
            dedupe: Some("agent:s".into()),
            created_at: 10,
            ..m.clone()
        };
        store.queue_session_message(&first).await.unwrap();
        store.resolve_session_message("f1", None, 11).await.unwrap();
        let again = SessionMessage { id: "f2".into(), created_at: 12, ..first.clone() };
        store.queue_session_message(&again).await.unwrap();
        let got = store.latest_session_message("sx", "agent:s").await.unwrap().unwrap();
        assert_eq!((got.id.as_str(), got.status.as_str()), ("f2", "waiting"));
        assert!(store.latest_session_message("sy", "agent:s").await.unwrap().is_none());

        let c = Connection {
            id: "c".into(),
            name: "alerts".into(),
            kind: "http".into(),
            url: String::new(),
            config: json!({}),
            has_secret: false,
            created_at: 1,
            updated_at: 1,
            last_error: None,
            last_error_at: None,
        };
        store.insert_connection(&c).await.unwrap();
        assert!(matches!(
            store.insert_connection(&Connection { id: "d".into(), ..c.clone() }).await,
            Err(StoreError::Conflict(_))
        ));
        store.update_connection(&Connection { has_secret: true, ..c.clone() }).await.unwrap();
        assert!(store.connection_by_name("alerts").await.unwrap().unwrap().has_secret);
        assert_eq!(store.list_connections().await.unwrap().len(), 1);
        // A failing connection changes once per new error, and once when it works again.
        assert!(store.set_connection_error("c", Some("401"), 5).await.unwrap());
        assert!(!store.set_connection_error("c", Some("401"), 6).await.unwrap(), "the same error again");
        let failing = store.get_connection("c").await.unwrap().unwrap();
        assert_eq!((failing.last_error.as_deref(), failing.last_error_at), (Some("401"), Some(5)));
        assert!(store.set_connection_error("c", Some("timed out"), 7).await.unwrap(), "a new reason");
        assert!(store.set_connection_error("c", None, 8).await.unwrap());
        assert!(!store.set_connection_error("c", None, 9).await.unwrap(), "already working");
        store.set_connection_error("c", Some("401"), 10).await.unwrap();
        store.update_connection(&c).await.unwrap();
        assert_eq!(store.get_connection("c").await.unwrap().unwrap().last_error, None, "saving it forgets the error");
        assert!(store.delete_connection("c").await.unwrap());
    }
}
