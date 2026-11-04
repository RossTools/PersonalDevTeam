//! Attempts: Start, ending, merge state and lookups.

use super::*;

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
    /// Finish the ticket once the branch is merged (Merge… on a pull request, PDT-75); with `finish_delete_branch`
    /// the branch goes with the worktree.
    pub finish_on_merge: bool,
    pub finish_delete_branch: bool,
    /// Its pull request was set to merge (auto-merge or the merge queue) at some point (PDT-104, sticky).
    pub merge_requested: bool,
    /// A workflow's merge was refused for something its agent can fix (PDT-139, sticky).
    pub merge_refused: bool,
    /// PDT named the branch from the project's template (true) or was given the name (false); None for attempts
    /// from before PDT recorded it (PDT-594).
    pub branch_named: Option<bool>,
}

const ATTEMPT_SELECT: &str = "SELECT a.id, a.ticket_id, a.branch, a.base_branch, a.worktree_id, a.state, a.created_at, \
                              a.ended_at, a.last_head, a.workspace_id FROM attempts a";
const ATTEMPT_INFO_SELECT: &str = "SELECT a.id, a.ticket_id, a.branch, a.base_branch, a.worktree_id, a.state, \
                                   a.created_at, a.ended_at, a.last_head, a.workspace_id, a.base_commit, a.own_commits, \
                                   a.finish_on_merge, a.finish_delete_branch, a.merge_requested, a.merge_refused, a.branch_named, t.project_id, COALESCE(p.key, '') AS pkey, t.number, p.repo_path, \
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
        finish_on_merge: r.try_get::<i64, _>("finish_on_merge")? != 0,
        finish_delete_branch: r.try_get::<i64, _>("finish_delete_branch")? != 0,
        merge_requested: r.try_get::<i64, _>("merge_requested")? != 0,
        merge_refused: r.try_get::<i64, _>("merge_refused")? != 0,
        branch_named: r.try_get::<Option<i64>, _>("branch_named")?.map(|n| n != 0),
    })
}

impl Store {
    /// Records a new active attempt on a ticket (Start). `Conflict` if the ticket already has one.
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

    /// Records whether PDT named the attempt's branch from the project's template (PDT-594), so Won't do knows the
    /// branch is PDT's to delete from the remote whatever its name.
    pub async fn set_attempt_branch_named(&self, attempt_id: &str, named: bool) -> Result<(), StoreError> {
        sqlx::query("UPDATE attempts SET branch_named = ? WHERE id = ?")
            .bind(named)
            .bind(attempt_id)
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
        let mut tx = self.write_tx().await?;
        let Some((_, key, stage)) = ticket_head(&mut tx, ticket_id).await? else {
            return Err(TicketError::NotFound("ticket"));
        };
        if has_active_attempt(&mut tx, ticket_id).await? {
            return Err(TicketError::Active(key));
        }
        let (prev_updated_at, prev_pipeline_stage): (i64, Option<String>) =
            sqlx::query_as("SELECT updated_at, pipeline_stage FROM tickets WHERE id = ?")
                .bind(ticket_id)
                .fetch_one(&mut *tx)
                .await?;
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
        // Started (by the user, or auto-start, which the API holds to tickets already let in): let in (PDT-201).
        admit(&mut tx, ticket_id, now).await?;
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
        // Their event-log copies wait for `announce_start` (once the session exists): a workflow that fired on
        // `attempt.started` for a Start that is then undone would run for an attempt that is gone.
        for (e,) in &event_ids {
            sqlx::query("DELETE FROM events WHERE key = ? AND source = 'tickets'").bind(e).execute(&mut *tx).await?;
        }
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
            prev_pipeline_stage: prev_pipeline_stage.as_deref().and_then(PipelineStage::parse),
            prev_updated_at,
            event_ids: event_ids.into_iter().map(|(e,)| e).collect(),
        })
    }

    /// Logs a `start_attempt`'s events (`ticket.stage_changed`, `attempt.started`) for workflows, once Start has
    /// what it makes (its session): until then a failed Start can still be undone without a workflow having seen it.
    pub async fn announce_start(&self, started: &StartedAttempt, now: i64) -> Result<(), StoreError> {
        self.announce_events(&started.event_ids, now).await
    }

    /// Starts left half-done (PDT-327): active attempts made between `since` and `before` whose `started` event never
    /// reached the event log (pdtd stopped, or the write failed, between the attempt and its announcement), each with
    /// the timeline events its start made (the stage change and `started`), to announce now.
    pub async fn unannounced_starts(&self, since: i64, before: i64) -> Result<Vec<(String, Vec<String>)>, StoreError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT a.ticket_id, a.created_at FROM attempts a \
             JOIN ticket_events e ON e.ticket_id = a.ticket_id AND e.kind = 'started' \
               AND json_extract(e.data, '$.attemptId') = a.id \
             WHERE a.state = 'active' AND a.created_at >= ? AND a.created_at < ? \
               AND NOT EXISTS (SELECT 1 FROM events v WHERE v.key = e.id AND v.source = 'tickets') \
             ORDER BY a.created_at",
        )
        .bind(since)
        .bind(before)
        .fetch_all(&self.pool)
        .await?;
        let mut out = vec![];
        for (ticket, at) in rows {
            let ids: Vec<(String,)> = sqlx::query_as(
                "SELECT id FROM ticket_events WHERE ticket_id = ? AND created_at = ? AND kind IN ('moved', 'started') \
                 ORDER BY id",
            )
            .bind(&ticket)
            .bind(at)
            .fetch_all(&self.pool)
            .await?;
            out.push((ticket, ids.into_iter().map(|(e,)| e).collect()));
        }
        Ok(out)
    }

    /// Logs timeline events for workflows (each once: the event log keeps one per timeline event).
    pub async fn announce_events(&self, event_ids: &[String], now: i64) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        for e in event_ids {
            let row = sqlx::query(
                "SELECT id, ticket_id, kind, actor, body, data, created_at FROM ticket_events WHERE id = ?",
            )
            .bind(e)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(ev) = row.as_ref().map(to_event).transpose()? else { continue };
            let row = crate::event_log::TimelineRow {
                id: &ev.id,
                ticket_id: &ev.ticket_id,
                kind: ev.kind,
                actor: ev.actor,
                body: ev.body.as_deref(),
                data: ev.data.as_ref(),
            };
            crate::event_log::mirror_timeline(&mut tx, row, now).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Takes back a `start_attempt` whose session could not be made: the attempt, its events and the stage change go;
    /// the ticket is as it was, pipeline stage too (the worktree is the caller's).
    pub async fn undo_start(&self, started: &StartedAttempt) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        sqlx::query("DELETE FROM attempts WHERE id = ?").bind(&started.attempt.id).execute(&mut *tx).await?;
        for e in &started.event_ids {
            sqlx::query("DELETE FROM ticket_events WHERE id = ?").bind(e).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE tickets SET stage = ?, pipeline_stage = ?, updated_at = ? WHERE id = ?")
            .bind(started.prev_stage.as_str())
            .bind(started.prev_pipeline_stage.map(PipelineStage::as_str))
            .bind(started.prev_updated_at)
            .bind(&started.ticket_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Ends an active attempt (None when it wasn't active). A merge records how it was found (`via`) and the PR, which
    /// the `attempt.merged` event carries.
    pub async fn end_attempt(
        &self,
        id: &str,
        state: AttemptState,
        by: impl Into<By>,
        via: Option<pdt_core::workflow::events::MergeVia>,
        pr: Option<u32>,
        now: i64,
    ) -> Result<Option<String>, StoreError> {
        let kind = match state {
            AttemptState::Merged => EventKind::Merged,
            AttemptState::Abandoned => EventKind::Abandoned,
            AttemptState::Active => return Ok(None),
        };
        let mut tx = self.write_tx().await?;
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
        let mut data = json!({"attemptId": id, "branch": branch, "baseBranch": base_branch});
        if state == AttemptState::Merged {
            data["via"] = json!(via.unwrap_or(pdt_core::workflow::events::MergeVia::Detected));
            data["pr"] = json!(pr);
        }
        insert_event(&mut tx, &ticket_id, kind, by, None, Some(data), now).await?;
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

    /// Whether to finish the ticket once the attempt's branch is merged, and whether to delete the branch then.
    /// Sets (or clears) a ticket's pipeline stage. False for an unknown ticket.
    pub async fn set_pipeline_stage(&self, ticket_id: &str, stage: Option<PipelineStage>) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE tickets SET pipeline_stage = ? WHERE id = ?")
            .bind(stage.map(PipelineStage::as_str))
            .bind(ticket_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// How many active attempts a project's tickets have (its running-attempt cap).
    pub async fn count_active_attempts(&self, project_id: &str) -> Result<u32, StoreError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM attempts a JOIN tickets t ON t.id = a.ticket_id \
             WHERE t.project_id = ? AND a.state = 'active'",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(u32::try_from(n).unwrap_or(0))
    }

    /// Whether an attempt of the project's tickets ever merged through a pull request (its `merged` event names one):
    /// the project's attempts use PRs, so a workflow never merges one of them locally (PDT-181).
    pub async fn project_merged_a_pr(&self, project_id: &str) -> Result<bool, StoreError> {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM ticket_events e JOIN tickets t ON t.id = e.ticket_id \
             WHERE t.project_id = ? AND e.kind = 'merged' AND json_extract(e.data, '$.pr') IS NOT NULL)",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n != 0)
    }

    pub async fn set_attempt_finish(&self, id: &str, finish: bool, delete_branch: bool) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE attempts SET finish_on_merge = ?, finish_delete_branch = ? WHERE id = ?")
            .bind(finish)
            .bind(finish && delete_branch)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Records that the attempt's pull request was set to merge; a refused merge (PDT-139) is forgotten, as this
    /// takes over. True when it was news.
    pub async fn set_attempt_merge_requested(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE attempts SET merge_requested = 1, merge_refused = 0 WHERE id = ? AND merge_requested = 0",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Records that a workflow's merge was refused for something the attempt's agent can fix (PDT-139). True when it
    /// was news. Not while it is set to merge (that says more).
    pub async fn set_attempt_merge_refused(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE attempts SET merge_refused = 1 WHERE id = ? AND merge_refused = 0 AND merge_requested = 0",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Forgets it, and a refused merge too (a person turned auto-merge off: PDT notices turn it on for neither). True
    /// when either was set.
    pub async fn clear_attempt_merge_requested(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE attempts SET merge_requested = 0, merge_refused = 0 WHERE id = ? AND \
             (merge_requested = 1 OR merge_refused = 1)",
        )
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
    /// The attempt working in a worktree: its active one, else the newest.
    pub async fn attempt_for_worktree(&self, worktree_id: &str) -> Result<Option<AttemptInfo>, StoreError> {
        let sql = format!(
            "{ATTEMPT_INFO_SELECT} WHERE a.worktree_id = ? ORDER BY a.state = 'active' DESC, a.created_at DESC LIMIT 1"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(worktree_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_attempt_info).transpose()
    }

    pub async fn active_attempt(&self, ticket_id: &str) -> Result<Option<AttemptInfo>, StoreError> {
        let sql = format!("{ATTEMPT_INFO_SELECT} WHERE a.ticket_id = ? AND a.state = 'active'");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(ticket_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_attempt_info).transpose()
    }

    /// Whether `branch` is one of PDT's in project `project_id`: an attempt's (in any state) or a worktree's.
    pub async fn is_pdt_branch(&self, project_id: &str, branch: &str) -> Result<bool, StoreError> {
        let n: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM attempts a JOIN tickets t ON t.id = a.ticket_id \
               WHERE t.project_id = ? AND a.branch = ?) \
             + (SELECT COUNT(*) FROM worktrees WHERE project_id = ? AND branch = ?)",
        )
        .bind(project_id)
        .bind(branch)
        .bind(project_id)
        .bind(branch)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Every active attempt, oldest first.
    pub async fn list_active_attempts(&self) -> Result<Vec<AttemptInfo>, StoreError> {
        let sql = format!("{ATTEMPT_INFO_SELECT} WHERE a.state = 'active' ORDER BY a.created_at, a.id");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_attempt_info).collect()
    }
}
