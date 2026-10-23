//! Review (Phase 6; docs/superpowers/specs/2026-10-19-review-design.md): a worktree's changes and the two sides of a
//! file, line comments, Send feedback into the coder session, Approve, and Merge (a local merge, or auto-merge of the
//! attempt's pull request on GitHub).

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::conversation::SessionMode;
use pdt_core::policy::{Merger, may_merge};
use pdt_core::pr::{Mergeable, PrState};
use pdt_core::project::Project;
use pdt_core::review::{
    ChangeStatus, ChangedFile, Changes, FeedbackItem, FeedbackResult, FileSides, MAX_COMMENT_BYTES, MAX_SIDE_BYTES,
    MergeMethod, MergeOutcome, MergePr, MergePreview, MergeProblem, MergeRequest, MergeResult, MergeStyle,
    NewReviewComment, ReviewComment, ReviewCommentPatch, conflict_message, feedback_message, feedback_record,
    merge_message, open_pr_message, squash_message,
};
use pdt_core::session::{Session, SessionKind};
use pdt_core::ticket::{Actor, AttemptState, EventKind, Ticket};
use pdt_core::worktree::Worktree;
use pdt_store::AttemptInfo;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

fn conflict(code: &'static str, message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, code, message)
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", message)
}

/// The worktree, its project and the attempt working in it; 409 `worktree_missing` when its folder is gone.
async fn load(st: &AppState, worktree_id: &str) -> Result<(Worktree, Project, Option<AttemptInfo>), ApiError> {
    let wt = st.store.get_worktree(worktree_id).await?.ok_or_else(|| ApiError::not_found("worktree"))?;
    let project = st.store.get_project(&wt.project_id).await?.ok_or_else(|| ApiError::not_found("project"))?;
    if !PathBuf::from(&wt.path).is_dir() {
        return Err(conflict("worktree_missing", "this worktree's folder is gone"));
    }
    let attempt = st.store.attempt_for_worktree(worktree_id).await?;
    Ok((wt, project, attempt))
}

/// The base branch and the fallback commit the changes are measured from.
fn base_of(wt: &Worktree, project: &Project, attempt: Option<&AttemptInfo>) -> (String, String) {
    match attempt {
        Some(a) => (a.attempt.base_branch.clone(), a.base_commit.clone()),
        None => (project.base_branch.clone(), wt.base_commit.clone()),
    }
}

fn to_status(s: pdt_git::FileStatus) -> ChangeStatus {
    match s {
        pdt_git::FileStatus::Added => ChangeStatus::Added,
        pdt_git::FileStatus::Modified => ChangeStatus::Modified,
        pdt_git::FileStatus::Deleted => ChangeStatus::Deleted,
        pdt_git::FileStatus::Renamed => ChangeStatus::Renamed,
    }
}

/// `GET /api/worktrees/{id}/changes`.
pub(crate) async fn changes(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Changes>, ApiError> {
    let (wt, project, attempt) = load(&st, &id).await?;
    let (base, fallback) = base_of(&wt, &project, attempt.as_ref());
    let path = PathBuf::from(&wt.path);
    let base_for_git = base.clone();
    let (merge_base, head, dirty, files) = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let mb = pdt_git::review_base(&path, &base_for_git, &fallback);
        let head = pdt_git::rev_parse(&path, "HEAD").unwrap_or_default();
        let dirty = pdt_git::is_dirty(&path)?;
        let files = pdt_git::diff_files(&path, &mb)?;
        Ok((mb, head, dirty, files))
    })
    .await
    .map_err(ApiError::internal)??;
    let approved = match &attempt {
        Some(a) => st
            .store
            .latest_event_data(&a.ticket_id, EventKind::Review)
            .await?
            .is_some_and(|d| d["attemptId"] == json!(a.attempt.id) && d["head"] == json!(head) && !head.is_empty()),
        None => false,
    };
    Ok(Json(Changes {
        worktree_id: wt.id,
        branch: wt.branch,
        base,
        merge_base,
        head,
        dirty,
        files: files
            .into_iter()
            .map(|f| ChangedFile {
                path: f.path,
                old_path: f.old_path,
                status: to_status(f.status),
                untracked: f.untracked,
                binary: f.binary,
                added: f.added,
                removed: f.removed,
                version: f.version,
            })
            .collect(),
        attempt_active: attempt.as_ref().is_some_and(|a| a.attempt.state == AttemptState::Active),
        attempt_id: attempt.as_ref().map(|a| a.attempt.id.clone()),
        ticket_id: attempt.map(|a| a.ticket_id),
        approved,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileQuery {
    path: String,
    #[serde(default)]
    old_path: Option<String>,
}

/// A side's bytes as text, or why not: (text, binary, too large).
fn side_text(bytes: Option<Vec<u8>>) -> (Option<String>, bool, bool) {
    match bytes {
        None => (None, false, false),
        Some(b) if b.len() > MAX_SIDE_BYTES => (None, false, true),
        Some(b) if b.iter().take(8000).any(|c| *c == 0) => (None, true, false),
        Some(b) => match String::from_utf8(b) {
            Ok(s) => (Some(s), false, false),
            Err(_) => (None, true, false),
        },
    }
}

/// `GET /api/worktrees/{id}/changes/file?path=&oldPath=`.
pub(crate) async fn file(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
) -> Result<Json<FileSides>, ApiError> {
    let (wt, project, attempt) = load(&st, &id).await?;
    let (base, fallback) = base_of(&wt, &project, attempt.as_ref());
    let rel = crate::files::clean_path(&q.path)?;
    let old_rel = match q.old_path.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => crate::files::clean_path(p)?,
        None => rel.clone(),
    };
    if rel.is_empty() || old_rel.is_empty() {
        return Err(invalid("name a file"));
    }
    let checkout = PathBuf::from(&wt.path);
    let sides = tokio::task::spawn_blocking(move || -> Result<FileSides, ApiError> {
        let mb = pdt_git::review_base(&checkout, &base, &fallback);
        let old = pdt_git::file_at(&checkout, &mb, &old_rel)?;
        let full = crate::files::contained(&checkout, &rel)?;
        let (new, new_huge) = match std::fs::metadata(&full) {
            Ok(m) if m.is_file() && m.len() > MAX_SIDE_BYTES as u64 => (None, true),
            Ok(m) if m.is_file() => (Some(std::fs::read(&full).map_err(ApiError::internal)?), false),
            _ => (None, false),
        };
        let (old, old_bin, old_big) = side_text(old);
        let (new, new_bin, new_big) = side_text(new);
        let new_big = new_big || new_huge;
        let (binary, too_large) = (old_bin || new_bin, old_big || new_big);
        let (old, new) = if binary || too_large { (None, None) } else { (old, new) };
        Ok(FileSides { path: rel, old, new, binary, too_large })
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(sides))
}

/// `GET /api/worktrees/{id}/comments`.
pub(crate) async fn list_comments(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ReviewComment>>, ApiError> {
    st.store.get_worktree(&id).await?.ok_or_else(|| ApiError::not_found("worktree"))?;
    Ok(Json(st.store.list_review_comments(&id).await?))
}

fn check_body(body: &str) -> Result<String, ApiError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(invalid("a comment needs text"));
    }
    if body.len() > MAX_COMMENT_BYTES {
        return Err(invalid("a comment can be at most 16 KB"));
    }
    Ok(body.to_string())
}

/// `POST /api/worktrees/{id}/comments`: a new draft. A reply takes its thread root's anchor.
pub(crate) async fn create_comment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<NewReviewComment>, JsonRejection>,
) -> Result<(StatusCode, Json<ReviewComment>), ApiError> {
    let Json(input) = payload?;
    Ok((StatusCode::CREATED, Json(add_comment(&st, &id, input, "user", false).await?)))
}

/// A line comment on a worktree's changes by `author` (`user`, or an agent's persona). `sent`: it is not a draft
/// (an agent's comment goes nowhere else).
pub(crate) async fn add_comment(
    st: &AppState,
    worktree_id: &str,
    input: NewReviewComment,
    author: &str,
    sent: bool,
) -> Result<ReviewComment, ApiError> {
    let id = worktree_id.to_string();
    st.store.get_worktree(&id).await?.ok_or_else(|| ApiError::not_found("worktree"))?;
    let body = check_body(&input.body)?;
    let now = now_ms();
    let mut c = ReviewComment {
        id: uuid::Uuid::now_v7().to_string(),
        worktree_id: id.clone(),
        parent_id: None,
        path: String::new(),
        side: input.side,
        line: input.line,
        end_line: input.end_line.unwrap_or(input.line),
        line_text: input.line_text,
        body,
        author: author.into(),
        sent_at: sent.then_some(now),
        session_id: None,
        resolved_at: None,
        created_at: now,
        updated_at: now,
    };
    match input.parent_id.as_deref().filter(|p| !p.is_empty()) {
        Some(pid) => {
            let parent = st.store.get_review_comment(pid).await?.filter(|p| p.worktree_id == id);
            let parent = parent.ok_or_else(|| invalid("the comment to reply to does not exist"))?;
            let root = match &parent.parent_id {
                Some(rid) => st.store.get_review_comment(rid).await?.unwrap_or(parent),
                None => parent,
            };
            c.parent_id = Some(root.id.clone());
            c.path = root.path;
            c.side = root.side;
            c.line = root.line;
            c.end_line = root.end_line;
            c.line_text = root.line_text;
        }
        None => {
            c.path = crate::files::clean_path(&input.path)?;
            if c.path.is_empty() {
                return Err(invalid("name the file the comment is on"));
            }
            if c.line == 0 || c.end_line < c.line {
                return Err(invalid("lines are numbered from 1, and the range must not run backwards"));
            }
        }
    }
    st.store.insert_review_comment(&c).await?;
    Ok(c)
}

async fn get_comment(st: &AppState, id: &str) -> Result<ReviewComment, ApiError> {
    st.store.get_review_comment(id).await?.ok_or_else(|| ApiError::not_found("comment"))
}

fn sent_already() -> ApiError {
    conflict("comment_sent", "this comment was already sent to the agent")
}

/// `PATCH /api/review-comments/{id}`: a draft's text, a thread's resolved state.
pub(crate) async fn update_comment(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<ReviewCommentPatch>, JsonRejection>,
) -> Result<Json<ReviewComment>, ApiError> {
    let Json(patch) = payload?;
    let mut c = get_comment(&st, &id).await?;
    let now = now_ms();
    if let Some(body) = &patch.body {
        if c.sent_at.is_some() {
            return Err(sent_already());
        }
        c.body = check_body(body)?;
    }
    if let Some(resolved) = patch.resolved {
        if c.parent_id.is_some() {
            return Err(invalid("only a thread's first comment is resolved"));
        }
        c.resolved_at = if resolved { c.resolved_at.or(Some(now)) } else { None };
    }
    c.updated_at = now;
    st.store.update_review_comment(&c).await?;
    Ok(Json(c))
}

/// `DELETE /api/review-comments/{id}`: drafts only (a root's replies go with it).
pub(crate) async fn delete_comment(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let c = get_comment(&st, &id).await?;
    if c.sent_at.is_some() {
        return Err(sent_already());
    }
    st.store.delete_review_comment(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Where review feedback goes: the newest agent session of the worktree's attempt (else of the worktree itself) that
/// can take a message now (a conversation, or a terminal whose PTY runs). `Err` names why there is none.
async fn coder_session(st: &AppState, wt: &Worktree, attempt: Option<&AttemptInfo>) -> Result<Session, ApiError> {
    let mut agents: Vec<Session> = st
        .store
        .list_sessions(&wt.project_id)
        .await?
        .into_iter()
        .filter(|s| s.kind == SessionKind::AgentTui)
        .filter(|s| match attempt {
            Some(a) if s.attempt_id.is_some() => s.attempt_id.as_deref() == Some(a.attempt.id.as_str()),
            _ => s.worktree_id.as_deref() == Some(wt.id.as_str()),
        })
        .collect();
    if agents.is_empty() {
        return Err(conflict("no_agent_session", "no agent session works on this branch; start one first"));
    }
    agents.sort_by_key(|s| s.created_at);
    agents
        .into_iter()
        .rev()
        .find(|s| s.mode == SessionMode::Conversation || st.ptys.is_running(&s.id))
        .ok_or_else(|| conflict("session_not_running", "the agent's terminal session isn't running; restart it first"))
}

/// Puts `text` into the session: a conversation's next message, or a bracketed paste into a terminal.
async fn deliver(st: &AppState, session: &Session, text: &str) -> Result<(), ApiError> {
    if session.mode == SessionMode::Conversation {
        crate::conversations::send_user_message(st, session, text).await
    } else {
        crate::activity::type_message(st, &session.id, text)
            .await
            .map_err(|e| conflict("session_not_running", format!("could not type into the session: {e}")))
    }
}

/// `POST /api/worktrees/{id}/feedback`: every draft in one message to the coder session; the drafts become sent.
pub(crate) async fn feedback(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<FeedbackResult>, ApiError> {
    let wt = st.store.get_worktree(&id).await?.ok_or_else(|| ApiError::not_found("worktree"))?;
    let attempt = st.store.attempt_for_worktree(&id).await?;
    let all = st.store.list_review_comments(&id).await?;
    let drafts: Vec<&ReviewComment> = all.iter().filter(|c| c.sent_at.is_none()).collect();
    if drafts.is_empty() {
        return Err(conflict("no_drafts", "there are no draft comments to send"));
    }
    let session = coder_session(&st, &wt, attempt.as_ref()).await?;
    let items: Vec<FeedbackItem> = drafts
        .iter()
        .map(|c| FeedbackItem {
            path: c.path.clone(),
            side: c.side,
            line: c.line,
            end_line: c.end_line,
            line_text: c.line_text.clone(),
            body: c.body.clone(),
            reply_to: c.parent_id.as_ref().and_then(|p| all.iter().find(|r| &r.id == p)).map(|r| r.body.clone()),
        })
        .collect();
    deliver(&st, &session, &feedback_message(&wt.branch, &items)).await?;
    let ids: Vec<String> = drafts.iter().map(|c| c.id.clone()).collect();
    let sent = st.store.mark_review_comments_sent(&ids, &session.id, now_ms()).await? as u32;
    if let Some(a) = &attempt {
        // The comments go with the worktree; the timeline keeps what was sent.
        let record = feedback_record(&items);
        let data = json!({"worktreeId": id, "sessionId": session.id, "count": sent});
        st.store
            .add_ticket_event(&a.ticket_id, EventKind::Feedback, Actor::User, Some(&record), Some(data), now_ms())
            .await?;
    }
    Ok(Json(FeedbackResult { session_id: session.id, sent }))
}

async fn load_attempt(st: &AppState, id: &str) -> Result<AttemptInfo, ApiError> {
    st.store.get_attempt(id).await?.ok_or_else(|| ApiError::not_found("attempt"))
}

async fn load_ticket(st: &AppState, id: &str) -> Result<Ticket, ApiError> {
    let t = st.store.get_ticket(id).await?.ok_or_else(|| ApiError::not_found("ticket"))?;
    crate::overlap::decorate(st, t).await
}

/// `POST /api/attempts/{id}/approve`: records the approval at the branch's head.
pub(crate) async fn approve(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Ticket>, ApiError> {
    let info = load_attempt(&st, &id).await?;
    if info.attempt.state != AttemptState::Active {
        return Err(conflict("attempt_inactive", "this attempt has ended"));
    }
    record_approval(&st, &info, json!({})).await?;
    Ok(Json(load_ticket(&st, &info.ticket_id).await?))
}

/// The branch's head commit, if it has one.
async fn branch_head(info: &AttemptInfo) -> Result<Option<String>, ApiError> {
    let repo = PathBuf::from(&info.repo_path);
    let branch = format!("refs/heads/{}", info.attempt.branch);
    tokio::task::spawn_blocking(move || pdt_git::rev_parse(&repo, &branch)).await.map_err(ApiError::internal)
}

/// Records the user's approval of the attempt at its branch head (Approve, or a gate's `approve` choice; `extra`
/// names the workflow and run).
pub(crate) async fn record_approval(
    st: &AppState,
    info: &AttemptInfo,
    extra: serde_json::Value,
) -> Result<(), ApiError> {
    let head = branch_head(info).await?;
    let mut data = json!({"attemptId": info.attempt.id, "verdict": "approved", "head": head});
    if let (Some(d), serde_json::Value::Object(x)) = (data.as_object_mut(), extra) {
        d.extend(x);
    }
    st.store.add_ticket_event(&info.ticket_id, EventKind::Review, Actor::User, None, Some(data), now_ms()).await?;
    Ok(())
}

/// Whether a person approved the attempt at its branch's current head.
async fn approved_at_head(st: &AppState, info: &AttemptInfo) -> Result<bool, ApiError> {
    let Some(head) = branch_head(info).await? else { return Ok(false) };
    Ok(st
        .store
        .latest_event_data(&info.ticket_id, EventKind::Review)
        .await?
        .is_some_and(|d| d["attemptId"] == json!(info.attempt.id) && d["head"] == json!(head)))
}

/// What Merge would do for the attempt now.
async fn preview(st: &AppState, info: &AttemptInfo) -> Result<MergePreview, ApiError> {
    let (branch, base) = (info.attempt.branch.clone(), info.attempt.base_branch.clone());
    let mut p = MergePreview {
        method: MergeMethod::Local,
        branch: branch.clone(),
        base: base.clone(),
        commits: 0,
        problem: None,
        conflicts: vec![],
        base_checkout: None,
        pr: None,
    };
    if info.attempt.state != AttemptState::Active {
        p.problem = Some(MergeProblem::Inactive);
        return Ok(p);
    }
    let pr = crate::github::watched_pr(st, &info.attempt.id).filter(|pr| pr.state == PrState::Open);
    if let Some(pr) = pr {
        let repo = crate::github::github_repo(&info.repo_path).await.unwrap_or_default();
        let gh = st.github.clone();
        let base_branch = pr.base_branch.clone();
        let queue = tokio::task::spawn_blocking(move || gh.merge_settings(&repo, &base_branch))
            .await
            .map_err(ApiError::internal)?
            .map(|s| s.queue)
            .unwrap_or(false);
        p.method = MergeMethod::Github;
        p.base = pr.base_branch.clone();
        p.pr = Some(MergePr { number: pr.number, url: pr.url.clone(), queue });
        p.problem = if pr.draft || pr.mergeable == Mergeable::Conflicting {
            Some(MergeProblem::PrNotMergeable)
        } else if pr.auto_merge || pr.in_merge_queue {
            Some(MergeProblem::PrQueued)
        } else {
            None
        };
        return Ok(p);
    }
    if st.store.get_project_settings(&info.project_id).await?.merge_via_pr {
        p.method = MergeMethod::Github;
        p.problem = Some(MergeProblem::NoPr);
        return Ok(p);
    }
    let repo = PathBuf::from(&info.repo_path);
    let worktree = info.worktree_path.clone().map(PathBuf::from);
    let (dirty, preview) = tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
        let dirty = match &worktree {
            Some(w) if w.is_dir() => pdt_git::is_dirty(w)?,
            _ => false,
        };
        Ok((dirty, pdt_git::merge_preview(&repo, &branch, &base)?))
    })
    .await
    .map_err(ApiError::internal)??;
    p.commits = preview.commits;
    p.conflicts = preview.conflicts;
    p.base_checkout = preview.base_checkout.map(|c| c.to_string_lossy().into_owned());
    p.problem = if dirty {
        Some(MergeProblem::Dirty)
    } else if p.commits == 0 {
        Some(MergeProblem::Nothing)
    } else if !p.conflicts.is_empty() {
        Some(MergeProblem::Conflict)
    } else if preview.base_dirty {
        Some(MergeProblem::BaseDirty)
    } else {
        None
    };
    Ok(p)
}

/// `GET /api/attempts/{id}/merge`.
pub(crate) async fn merge_preview(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MergePreview>, ApiError> {
    let info = load_attempt(&st, &id).await?;
    Ok(Json(preview(&st, &info).await?))
}

fn why_blocked(problem: MergeProblem) -> &'static str {
    match problem {
        MergeProblem::Inactive => "this attempt has ended",
        MergeProblem::Dirty => "the worktree has uncommitted changes; commit or discard them first",
        MergeProblem::Conflict => "the merge conflicts; merge the base branch into the branch first",
        MergeProblem::BaseDirty => "the checkout that has the base branch has uncommitted changes",
        MergeProblem::Nothing => "the branch has no commits the base branch lacks",
        MergeProblem::PrNotMergeable => "GitHub can't merge the pull request (a draft, or it conflicts)",
        MergeProblem::PrQueued => "auto-merge is already on for the pull request",
        MergeProblem::NoPr => "this project merges through pull requests, and the branch has no open one yet",
    }
}

/// Why a merge didn't happen: the project's policy, or something in its way. `fixable`: the attempt's agent can clear
/// it with a commit (a conflict, uncommitted changes in the worktree), so a workflow can send it back to be fixed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Refusal {
    pub code: &'static str,
    pub reason: String,
    pub fixable: bool,
}

impl Refusal {
    fn blocked(problem: MergeProblem, fixable: bool) -> Self {
        Self { code: "merge_blocked", reason: why_blocked(problem).into(), fixable }
    }

    fn into_error(self) -> ApiError {
        conflict(self.code, self.reason)
    }
}

/// `POST /api/attempts/{id}/merge`: merges locally (the attempt ends as merged) or turns on auto-merge for its PR.
pub(crate) async fn merge(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<MergeResult>, ApiError> {
    // The body is optional: empty means a merge commit.
    let input: MergeRequest = if body.iter().all(u8::is_ascii_whitespace) {
        MergeRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_input", e.to_string()))?
    };
    let merger = match input.by.as_deref() {
        Some("user") => Merger::User,
        None | Some("agent") => Merger::Automated,
        Some(other) => return Err(invalid(format!("by is user or agent, not {other}"))),
    };
    merge_attempt(&st, &id, &input, merger).await?.map(Json).map_err(Refusal::into_error)
}

/// Records that the attempt's pull request was set to merge, and what to do once it merges (not given, a merge again
/// (PDT-104): what the earlier merge asked stays).
async fn record_merge(st: &AppState, info: &AttemptInfo, input: &MergeRequest) -> Result<(), ApiError> {
    if let Some(finish) = input.finish_on_merge {
        st.store.set_attempt_finish(&info.attempt.id, finish, input.delete_branch.unwrap_or(false)).await?;
    }
    st.store.set_attempt_merge_requested(&info.attempt.id).await?;
    Ok(())
}

/// The Merge command (the route, and the `attempt.merge` node as an automated merger): the project's policy, then
/// what would stop the merge, then the merge. A refusal is `Ok(Err(_))`; an automated merge refused for a pull request
/// that conflicts still records the request to merge (PDT-124), so PR notices gets it merging again once it is fixed.
pub(crate) async fn merge_attempt(
    st: &AppState,
    id: &str,
    input: &MergeRequest,
    merger: Merger,
) -> Result<Result<MergeResult, Refusal>, ApiError> {
    let st = st.clone();
    let info = load_attempt(&st, id).await?;
    let policy = st.store.get_project_settings(&info.project_id).await?.policy;
    let ticket = st.store.get_ticket_summary(&info.ticket_id).await?;
    let labels = ticket.as_ref().map(|t| t.labels.as_slice()).unwrap_or_default();
    let approved = merger == Merger::Automated && policy.needs_approval(labels) && approved_at_head(&st, &info).await?;
    if let Err(refusal) = may_merge(&policy, merger, approved, labels) {
        let reason = refusal.message(&policy).into();
        return Ok(Err(Refusal { code: refusal.code(), reason, fixable: false }));
    }
    let p = preview(&st, &info).await?;
    let automated = merger == Merger::Automated;
    match p.problem {
        None => {}
        // Already set to merge (a person, or PR notices, got there first): for a workflow that is what it asked.
        Some(MergeProblem::PrQueued) if automated => {
            record_merge(&st, &info, input).await?;
            return Ok(Ok(MergeResult { outcome: MergeOutcome::Queued, commit: None }));
        }
        Some(MergeProblem::PrNotMergeable) => {
            let draft = crate::github::watched_pr(&st, &info.attempt.id).is_none_or(|pr| pr.draft);
            if draft {
                let reason = "the pull request is a draft; mark it ready for review first".into();
                return Ok(Err(Refusal { code: "merge_blocked", reason, fixable: false }));
            }
            if automated {
                record_merge(&st, &info, input).await?;
            }
            let reason = "the pull request conflicts with its base; merge the base branch in, resolve the conflicts \
                          and push"
                .into();
            return Ok(Err(Refusal { code: "merge_blocked", reason, fixable: true }));
        }
        Some(problem) => {
            let fixable = matches!(problem, MergeProblem::Conflict | MergeProblem::Dirty);
            return Ok(Err(Refusal::blocked(problem, fixable)));
        }
    }
    if let Some(pr) = &p.pr {
        let repo = crate::github::github_repo(&info.repo_path).await.unwrap_or_default();
        let (gh, base, number) = (st.github.clone(), p.base.clone(), pr.number);
        tokio::task::spawn_blocking(move || {
            let flag = match gh.merge_settings(&repo, &base) {
                Ok(s) => pdt_core::pr::merge_flag(s.squash, s.merge, s.rebase, s.queue),
                // Unknown: squash (with a merge queue gh only notes that the queue sets the method).
                Err(_) => Some("--squash"),
            };
            gh.merge_pr(&repo, number, flag)
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| conflict("merge_failed", e.to_string()))?;
        record_merge(&st, &info, input).await?;
        crate::github::refresh_attempt(&st, &info).await;
        return Ok(Ok(MergeResult { outcome: MergeOutcome::Queued, commit: None }));
    }
    let (key, title) = ticket.map(|t| (t.key, t.title)).unwrap_or_default();
    let repo = PathBuf::from(&info.repo_path);
    let (branch, base) = (info.attempt.branch.clone(), info.attempt.base_branch.clone());
    let squash = input.style == Some(MergeStyle::Squash);
    let message = if squash { squash_message(&key, &title, &branch) } else { merge_message(&branch, &title) };
    let outcome = tokio::task::spawn_blocking(move || pdt_git::merge_local(&repo, &branch, &base, &message, squash))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| conflict("merge_failed", e.to_string()))?;
    let commit = match outcome {
        pdt_git::MergeOutcome::Merged { commit } => commit,
        pdt_git::MergeOutcome::Conflict { .. } => return Ok(Err(Refusal::blocked(MergeProblem::Conflict, true))),
        pdt_git::MergeOutcome::BaseDirty { .. } => return Ok(Err(Refusal::blocked(MergeProblem::BaseDirty, false))),
        pdt_git::MergeOutcome::Nothing => return Ok(Err(Refusal::blocked(MergeProblem::Nothing, false))),
    };
    let repo = PathBuf::from(&info.repo_path);
    let branch_ref = format!("refs/heads/{}", info.attempt.branch);
    if let Ok(Some(head)) = tokio::task::spawn_blocking(move || pdt_git::rev_parse(&repo, &branch_ref)).await {
        let _ = st.store.set_attempt_head(&info.attempt.id, &head, true).await;
    }
    let pr = crate::github::watched_pr(&st, &info.attempt.id).map(|p| p.number);
    let via = Some(pdt_core::workflow::events::MergeVia::MergedHere);
    let actor = if merger == Merger::User { Actor::User } else { Actor::System };
    st.store.end_attempt(&info.attempt.id, AttemptState::Merged, actor, via, pr, now_ms()).await?;
    Ok(Ok(MergeResult { outcome: MergeOutcome::Merged, commit: Some(commit) }))
}

/// `POST /api/attempts/{id}/ask-agent`: asks the coder session to clear what stops the merge: merge the base in and
/// resolve the conflicts, or open a pull request (a project that merges through them).
pub(crate) async fn ask_agent(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<FeedbackResult>, ApiError> {
    let info = load_attempt(&st, &id).await?;
    let p = preview(&st, &info).await?;
    let text = match p.problem {
        Some(MergeProblem::Conflict) => conflict_message(&p.branch, &p.base, &p.conflicts),
        Some(MergeProblem::NoPr) => open_pr_message(&p.branch, &p.base),
        _ => return Err(conflict("nothing_to_ask", "nothing the agent can fix stops this merge")),
    };
    let wid = info.attempt.worktree_id.clone().ok_or_else(|| conflict("worktree_missing", "the worktree is gone"))?;
    let wt = st.store.get_worktree(&wid).await?.ok_or_else(|| conflict("worktree_missing", "the worktree is gone"))?;
    let session = coder_session(&st, &wt, Some(&info)).await?;
    deliver(&st, &session, &text).await?;
    Ok(Json(FeedbackResult { session_id: session.id, sent: 0 }))
}
