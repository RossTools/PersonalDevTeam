//! GitLab as a [`Forge`] (PDT-246): merge requests are change requests (numbered by their `iid`), notes and
//! discussions their comments and review replies, approvals (and reviewers' "requested changes") their reviews; issues
//! and releases; auto-merge ("merge when the pipeline succeeds", every tier), and merge trains where a project has
//! them on (Premium and Ultimate, with merged-results pipelines): then the train is the base branch's merge queue.

use crate::api::{GitlabApi, GlError, project_path, s};
use pdt_core::brand::SHORT_NAME;
use pdt_core::pr::{Mergeable, PrState, QueueRemoval, QueueRemovalReason};
use pdt_forge::transport::encode;
use pdt_forge::{
    Capabilities, ChangeComment, ChangeFile, ChangeRequest, ChangeReview, ChangeStats, CloseReason, Forge, ForgeError,
    ForgeNotes, Issue, IssueDetail, MergeSettings, MergeStrategy, NewChangeRequest, NewIssue, OpenedChangeRequest,
    Release, RepoChangeRequest, ReviewVerdict,
};
use serde_json::{Value, json};

/// What GitLab can do: everything. Merge trains are found per project ([`Forge::merge_settings`]); the capability
/// says only that a base branch can have one.
pub const CAPABILITIES: Capabilities = Capabilities::ALL;

/// The longest comment PDT sends.
pub const MAX_COMMENT: usize = 60_000;

/// Items per page of a list PDT reads back to an earlier point (issues, merge requests), and the most pages.
pub const PAGE: usize = 100;
pub const PAGES: u32 = 10;

/// GitLab's part of agents' instructions: PDT's forge tools (pdt-mcp), not the GitLab CLI.
pub fn agent_notes() -> ForgeNotes {
    ForgeNotes {
        ci: Some(format!(
            "Read CI with {SHORT_NAME}'s tools: `ci_runs` (the pipelines on {{branch}} and their failed jobs) and \
             `ci_failed_logs` (the failed jobs' logs), or from the ticket's attempt (`pr.ci`: state, URL and runs, \
             when {SHORT_NAME} has seen the merge request). `change_request_view` shows the merge request with its \
             comments and approvals, `issue_view` an issue. Use these instead of the GitLab CLI (`glab`)."
        )),
        reply: Some("`change_request_comment`, with `replyTo` to answer a comment in a discussion".into()),
        watch: None,
    }
}

/// The ref a merge train's pipeline for merge request `iid` runs on.
pub fn train_ref(iid: u32) -> String {
    format!("refs/merge-requests/{iid}/train")
}

fn comment_body(body: &str) -> String {
    if body.chars().count() <= MAX_COMMENT {
        return body.to_string();
    }
    let mut out: String = body.chars().take(MAX_COMMENT - 1).collect();
    out.push('…');
    out
}

fn u32_of(v: &Value) -> Option<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn labels(v: &Value) -> Vec<String> {
    v["labels"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| l.as_str().or(l["name"].as_str()).map(String::from)).collect())
        .unwrap_or_default()
}

/// A bot account: GitLab's own (`bot: true`), a project's or group's access token (`project_12_bot_…`), or one named
/// so.
pub fn is_bot(user: &Value) -> bool {
    let name = user["username"].as_str().unwrap_or_default();
    user["bot"].as_bool() == Some(true)
        || ((name.starts_with("project_") || name.starts_with("group_")) && name.contains("_bot"))
        || name.ends_with("-bot")
        || name.ends_with("[bot]")
}

/// One merge request (a list's item, or `GET …/merge_requests/:iid`).
pub fn parse_mr(v: &Value) -> ChangeRequest {
    let state = match v["state"].as_str() {
        Some("opened") => PrState::Open,
        Some("merged") => PrState::Merged,
        _ => PrState::Closed,
    };
    let detailed = v["detailed_merge_status"].as_str();
    let mergeable = if v["has_conflicts"].as_bool() == Some(true) || detailed == Some("conflict") {
        Mergeable::Conflicting
    } else {
        match (detailed, v["merge_status"].as_str()) {
            (Some("checking" | "unchecked" | "preparing" | "approvals_syncing"), _) => Mergeable::Unknown,
            (Some(_), _) | (None, Some("can_be_merged")) => Mergeable::Mergeable,
            (None, Some("cannot_be_merged")) => Mergeable::Conflicting,
            _ => Mergeable::Unknown,
        }
    };
    ChangeRequest {
        number: u32_of(&v["iid"]).unwrap_or_default(),
        url: s(v, "web_url"),
        title: s(v, "title"),
        state,
        draft: v["draft"].as_bool().or(v["work_in_progress"].as_bool()).unwrap_or(false),
        head_sha: s(v, "sha"),
        base_branch: s(v, "target_branch"),
        mergeable,
        behind: detailed == Some("need_rebase"),
        auto_merge: v["merge_when_pipeline_succeeds"].as_bool().unwrap_or(false),
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
        created_at: v["created_at"].as_str().and_then(pdt_core::conversation::devin::iso_ms),
    }
}

/// The merge requests of a `source_branch` list: the newest open one from the project itself (not a fork's of the
/// same branch name), else its newest.
pub fn pick_mr(items: &[Value]) -> Option<&Value> {
    let own: Vec<&Value> = items
        .iter()
        .filter(|v| v["source_project_id"].is_null() || v["source_project_id"] == v["project_id"])
        .collect();
    own.iter().find(|v| v["state"] == "opened").or_else(|| own.first()).copied()
}

/// Whether a merge-train car (`GET …/merge_trains/merge_requests/:iid`) is still on the train.
pub fn car_on_train(v: &Value) -> bool {
    matches!(v["status"].as_str(), Some("idle" | "stale" | "fresh" | "merging"))
}

/// The newest merge-train system note of a merge request's notes (newest first), when it took the merge request off
/// the train: "removed this merge request from the merge train because the pipeline did not succeed".
pub fn train_removal(notes: &[Value]) -> Option<QueueRemoval> {
    let n = notes
        .iter()
        .find(|n| n["system"].as_bool() == Some(true) && s(n, "body").to_ascii_lowercase().contains("merge train"))?;
    let body = s(n, "body").to_ascii_lowercase();
    if !(body.contains("removed") || body.contains("aborted")) {
        return None;
    }
    let why = body.split_once("because").map(|(_, w)| w.trim().trim_end_matches('.').to_string());
    let reason = match why.as_deref() {
        Some(w) if w.contains("conflict") => QueueRemovalReason::Conflict,
        Some(w) if w.contains("pipeline") || w.contains("fail") => QueueRemovalReason::FailedChecks,
        Some(w) if w.contains("merged") => QueueRemovalReason::Merged,
        Some(w) => QueueRemovalReason::Other(w.to_string()),
        None => QueueRemovalReason::Other("removed by hand".into()),
    };
    Some(QueueRemoval { id: format!("note:{}", n["id"]), reason, by_queue: why.is_some() })
}

/// A merge request of a project's list.
fn repo_mr(api: &GitlabApi, repo: &str, v: &Value) -> Option<RepoChangeRequest> {
    let state = match v["state"].as_str() {
        Some("opened") => PrState::Open,
        Some("merged") => PrState::Merged,
        _ => PrState::Closed,
    };
    let own = v["source_project_id"].is_null() || v["source_project_id"] == v["project_id"];
    Some(RepoChangeRequest {
        number: u32_of(&v["iid"])?,
        title: s(v, "title"),
        body: s(v, "description"),
        url: s(v, "web_url"),
        author: s(&v["author"], "username"),
        is_bot: is_bot(&v["author"]),
        collaborator: api.insider(repo, &v["author"]),
        // A fork is named by its project id: the list doesn't give its path.
        head_repo: Some(if own { repo.to_string() } else { format!("project {}", v["source_project_id"]) }),
        head_branch: s(v, "source_branch"),
        head_sha: s(v, "sha"),
        base_branch: s(v, "target_branch"),
        labels: labels(v),
        state,
        draft: v["draft"].as_bool().unwrap_or(false),
        created_at: s(v, "created_at"),
        updated_at: s(v, "updated_at"),
    })
}

/// An issue (not a merge request).
pub fn parse_issue(v: &Value) -> Option<Issue> {
    Some(Issue {
        number: v["iid"].as_u64()?,
        title: s(v, "title"),
        body: s(v, "description"),
        url: s(v, "web_url"),
        author: s(&v["author"], "username"),
        labels: labels(v),
        created_at: s(v, "created_at"),
    })
}

/// A diff of `GET …/merge_requests/:iid/diffs`, with its added and deleted lines counted from its hunks.
pub fn parse_diff(v: &Value) -> Option<ChangeFile> {
    let path = s(v, "new_path");
    if path.is_empty() {
        return None;
    }
    let flag = |k: &str| v[k].as_bool() == Some(true);
    let status = if flag("new_file") {
        "added"
    } else if flag("deleted_file") {
        "removed"
    } else if flag("renamed_file") {
        "renamed"
    } else {
        "modified"
    };
    let diff = v["diff"].as_str().filter(|d| !d.is_empty() && !flag("too_large") && !flag("collapsed"));
    let (mut additions, mut deletions) = (0, 0);
    for l in diff.unwrap_or_default().lines() {
        if l.starts_with('+') && !l.starts_with("+++") {
            additions += 1;
        } else if l.starts_with('-') && !l.starts_with("---") {
            deletions += 1;
        }
    }
    Some(ChangeFile {
        status: status.into(),
        additions,
        deletions,
        patch: diff.map(String::from),
        previous_path: flag("renamed_file").then(|| s(v, "old_path")),
        path,
    })
}

/// A release; one not out yet (`upcoming_release`) is none.
pub fn parse_release(v: &Value) -> Result<Option<Release>, GlError> {
    let tag = s(v, "tag_name");
    if tag.is_empty() {
        return Err(GlError::BadAnswer("a release without a tag".into()));
    }
    if v["upcoming_release"].as_bool() == Some(true) {
        return Ok(None);
    }
    Ok(Some(Release {
        name: Some(s(v, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| tag.clone()),
        url: v["_links"]["self"].as_str().map(String::from).unwrap_or_default(),
        published_at: v["released_at"].as_str().map(String::from),
        notes: s(v, "description"),
        tag,
    }))
}

impl GitlabApi {
    fn mr_path(repo: &str, iid: u32) -> String {
        format!("{}/merge_requests/{iid}", project_path(repo))
    }

    /// A note's page: `<web>/<repo>/-/merge_requests/<iid>#note_<id>` (or `issues`).
    fn note_url(&self, repo: &str, what: &str, iid: u64, note: &Value) -> String {
        format!("{}/{repo}/-/{what}/{iid}#note_{}", self.web(), note["id"])
    }

    fn mr_url(&self, repo: &str, iid: u32) -> String {
        format!("{}/{repo}/-/merge_requests/{iid}", self.web())
    }

    /// The comments of `discussions` (system notes left out), as change request comments.
    fn discussion_comments(&self, repo: &str, iid: u32, discussions: &[Value]) -> Vec<ChangeComment> {
        discussions
            .iter()
            .flat_map(|d| d["notes"].as_array().cloned().unwrap_or_default())
            .filter(|n| n["system"].as_bool() != Some(true))
            .filter_map(|n| {
                let diff = n["type"] == "DiffNote";
                Some(ChangeComment {
                    id: n["id"].as_u64()?,
                    author: s(&n["author"], "username"),
                    is_bot: is_bot(&n["author"]),
                    body: s(&n, "body"),
                    url: self.note_url(repo, "merge_requests", iid.into(), &n),
                    path: diff.then(|| n["position"]["new_path"].as_str().map(String::from)).flatten(),
                    line: diff.then(|| u32_of(&n["position"]["new_line"])).flatten(),
                    in_review: diff,
                    collaborator: self.insider(repo, &n["author"]),
                })
            })
            .collect()
    }

    fn post_note(&self, repo: &str, what: &str, iid: u64, body: &str) -> Result<String, GlError> {
        let v = self.send_json(
            "POST",
            &format!("{}/{what}/{iid}/notes", project_path(repo)),
            json!({"body": comment_body(body)}),
        )?;
        Ok(self.note_url(repo, what, iid, &v))
    }

    /// Whether merge request `cr` is on its base's merge train, and where; else the train's last word on it.
    fn fill_train(&self, repo: &str, cr: &mut ChangeRequest) {
        let path = format!("{}/merge_trains/merge_requests/{}", project_path(repo), cr.number);
        match self.get(&path) {
            Ok(car) if car_on_train(&car) => {
                cr.in_merge_queue = true;
                let train = format!(
                    "{}/merge_trains/{}?scope=active&sort=asc&per_page=100",
                    project_path(repo),
                    encode(&cr.base_branch)
                );
                if let Ok(Value::Array(cars)) = self.get(&train) {
                    cr.queue_position = cars
                        .iter()
                        .position(|c| u32_of(&c["merge_request"]["iid"]) == Some(cr.number))
                        .and_then(|p| u32::try_from(p + 1).ok());
                }
                return;
            }
            Ok(_) | Err(GlError::Status(404, _)) => {}
            Err(e) => return tracing::debug!(repo, iid = cr.number, error = %e, "the merge train could not be read"),
        }
        let notes = format!("{}/notes?sort=desc&order_by=created_at&per_page=20", Self::mr_path(repo, cr.number));
        if let Ok(Value::Array(notes)) = self.get(&notes) {
            cr.queue_removal = train_removal(&notes);
        }
    }

    fn merge_now(
        &self,
        repo: &str,
        iid: u32,
        strategy: Option<MergeStrategy>,
        head: Option<&str>,
    ) -> Result<(), GlError> {
        let trains = self.project(repo).map(|p| p.merge_trains).unwrap_or(false);
        // `auto_merge` from GitLab 17.11, `merge_when_pipeline_succeeds` / `when_pipeline_succeeds` before it.
        let mut body = json!({"auto_merge": true});
        if trains {
            body["when_pipeline_succeeds"] = json!(true);
        } else {
            body["merge_when_pipeline_succeeds"] = json!(true);
        }
        if let Some(s) = strategy {
            body["squash"] = json!(s == MergeStrategy::Squash);
        }
        if let Some(h) = head {
            body["sha"] = json!(h);
        }
        let r = if trains {
            self.send_json("POST", &format!("{}/merge_trains/merge_requests/{iid}", project_path(repo)), body)
        } else {
            self.send_json("PUT", &format!("{}/merge", Self::mr_path(repo, iid)), body)
        };
        match r {
            Ok(_) => Ok(()),
            Err(GlError::Status(409, m)) => Err(GlError::Status(409, format!("the head of !{iid} moved: {m}"))),
            Err(GlError::Status(code @ (405 | 406 | 422), m)) => {
                Err(GlError::Status(code, format!("GitLab can't merge !{iid} now ({m})")))
            }
            Err(e) => Err(e),
        }
    }
}

impl Forge for GitlabApi {
    fn name(&self) -> &'static str {
        "GitLab"
    }
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }
    fn agent_notes(&self) -> ForgeNotes {
        agent_notes()
    }
    fn queue_name(&self) -> &'static str {
        "merge train"
    }
    fn queue_ref(&self, _base: &str, number: u32) -> Option<String> {
        Some(train_ref(number))
    }
    fn login_needed(&self) -> bool {
        (self.account().is_none() && self.wanted()) || self.rejected_at().is_some()
    }
    fn login_rejected_at(&self) -> Option<i64> {
        self.rejected_at()
    }
    fn change_request(&self, repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError> {
        let path = format!(
            "{}/merge_requests?source_branch={}&state=all&order_by=created_at&sort=desc&per_page=10",
            project_path(repo),
            encode(branch)
        );
        let items = match self.get(&path)? {
            Value::Array(a) => a,
            _ => return Err(GlError::BadAnswer("merge requests are not an array".into()).into()),
        };
        let Some(mut cr) = pick_mr(&items).map(parse_mr) else { return Ok(None) };
        if cr.state == PrState::Open && self.project(repo).is_ok_and(|p| p.merge_trains) {
            self.fill_train(repo, &mut cr);
        }
        Ok(Some(cr))
    }
    fn change_request_number(&self, repo: &str, number: u32) -> Result<Option<ChangeRequest>, ForgeError> {
        match self.get(&Self::mr_path(repo, number)) {
            Ok(v) => {
                let mut cr = parse_mr(&v);
                if cr.state == PrState::Open && self.project(repo).is_ok_and(|p| p.merge_trains) {
                    self.fill_train(repo, &mut cr);
                }
                Ok(Some(cr))
            }
            Err(GlError::Status(404, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn open_change_request(&self, repo: &str, new: &NewChangeRequest) -> Result<OpenedChangeRequest, ForgeError> {
        let title = if new.draft { format!("Draft: {}", new.title) } else { new.title.clone() };
        let body = json!({
            "source_branch": new.head,
            "target_branch": new.base,
            "title": title,
            "description": comment_body(&new.body),
        });
        match self.send_json("POST", &format!("{}/merge_requests", project_path(repo)), body) {
            Ok(v) => {
                let number =
                    u32_of(&v["iid"]).ok_or_else(|| GlError::BadAnswer("no merge request iid in the answer".into()))?;
                Ok(OpenedChangeRequest { number, url: s(&v, "web_url") })
            }
            Err(GlError::Status(code @ (400 | 409 | 422), m)) => Err(ForgeError::Failed(format!(
                "GitLab refused the merge request ({code}: {m}). Is {} pushed, with commits {} doesn't have?",
                new.head, new.base
            ))),
            Err(e) => Err(e.into()),
        }
    }
    /// Squash unless the project never squashes; a merge commit with the `merge` method, else a rebase (`ff` and
    /// `rebase_merge`); the merge train as the queue.
    fn merge_settings(&self, repo: &str, _base: &str) -> Result<MergeSettings, ForgeError> {
        let p = self.project(repo)?;
        Ok(MergeSettings {
            squash: p.squash_option != "never",
            merge: p.merge_method == "merge",
            rebase: p.merge_method != "merge",
            queue: p.merge_trains,
            // Merge when the pipeline succeeds (auto-merge) is always there.
            auto_merge: true,
        })
    }
    fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError> {
        self.merge_head(repo, number, strategy, None)
    }
    /// GitLab's `sha`: it refuses (409) once the head has moved.
    fn merge_head(
        &self,
        repo: &str,
        number: u32,
        strategy: Option<MergeStrategy>,
        head: Option<&str>,
    ) -> Result<(), ForgeError> {
        Ok(self.merge_now(repo, number, strategy, head)?)
    }
    fn viewer_login(&self) -> Result<String, ForgeError> {
        let login = s(&self.get("user")?, "username");
        if login.is_empty() { Err(GlError::BadAnswer("no username".into()).into()) } else { Ok(login) }
    }
    fn change_comments(&self, repo: &str, number: u32) -> Result<Vec<ChangeComment>, ForgeError> {
        let d = self.get_pages(&format!("{}/discussions?per_page=100", Self::mr_path(repo, number)))?;
        Ok(self.discussion_comments(repo, number, &d))
    }
    fn comment_change_request(&self, repo: &str, number: u32, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_note(repo, "merge_requests", number.into(), body)?)
    }
    /// Into the discussion the comment is in.
    fn reply_review_comment(&self, repo: &str, number: u32, comment_id: u64, body: &str) -> Result<String, ForgeError> {
        let mr = Self::mr_path(repo, number);
        let discussions = self.get_pages(&format!("{mr}/discussions?per_page=100"))?;
        let id = discussions
            .iter()
            .find(|d| d["notes"].as_array().is_some_and(|n| n.iter().any(|n| n["id"].as_u64() == Some(comment_id))))
            .and_then(|d| d["id"].as_str())
            .ok_or_else(|| ForgeError::Failed(format!("!{number} has no comment {comment_id} to reply to")))?;
        let v = self.send_json("POST", &format!("{mr}/discussions/{id}/notes"), json!({"body": comment_body(body)}))?;
        Ok(self.note_url(repo, "merge_requests", number.into(), &v))
    }
    /// Approvals, and reviewers who requested changes or reviewed without approving (GitLab 17+).
    fn change_reviews(&self, repo: &str, number: u32) -> Result<Vec<ChangeReview>, ForgeError> {
        let mr = Self::mr_path(repo, number);
        let url = self.mr_url(repo, number);
        let review = |u: &Value, state: &str| {
            Some(ChangeReview {
                id: u["id"].as_u64()?,
                author: s(u, "username"),
                is_bot: is_bot(u),
                state: state.into(),
                body: String::new(),
                url: url.clone(),
                collaborator: self.insider(repo, u),
            })
        };
        let approvals = self.get(&format!("{mr}/approvals"))?;
        let mut out: Vec<ChangeReview> = approvals["approved_by"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| review(&a["user"], "approved"))
            .collect();
        // Older servers have no reviewer states: approvals alone.
        if let Ok(Value::Array(reviewers)) = self.get(&format!("{mr}/reviewers")) {
            for r in &reviewers {
                let state = match r["state"].as_str() {
                    Some("requested_changes") => "changes_requested",
                    Some("reviewed") => "commented",
                    _ => continue,
                };
                if !out.iter().any(|x| Some(x.id) == r["user"]["id"].as_u64()) {
                    out.extend(review(&r["user"], state));
                }
            }
        }
        Ok(out)
    }
    fn recent_issues(&self, repo: &str, after: Option<u64>) -> Result<Vec<Issue>, ForgeError> {
        let mut out = vec![];
        for n in 1..=PAGES {
            let path = format!(
                "{}/issues?scope=all&state=all&order_by=created_at&sort=desc&per_page={PAGE}&page={n}",
                project_path(repo)
            );
            let items = match self.get(&path)? {
                Value::Array(a) => a,
                _ => return Err(GlError::BadAnswer("issues are not an array".into()).into()),
            };
            let lowest = items.iter().filter_map(|v| v["iid"].as_u64()).min();
            out.extend(items.iter().filter_map(parse_issue));
            let reached = after.is_none_or(|a| lowest.is_none_or(|l| l <= a));
            if reached || items.len() < PAGE {
                break;
            }
        }
        Ok(out)
    }
    fn recent_change_requests(&self, repo: &str, since: Option<&str>) -> Result<Vec<RepoChangeRequest>, ForgeError> {
        let mut out = vec![];
        for n in 1..=PAGES {
            let path = format!(
                "{}/merge_requests?scope=all&state=all&order_by=updated_at&sort=desc&per_page={PAGE}&page={n}",
                project_path(repo)
            );
            let items = match self.get(&path)? {
                Value::Array(a) => a,
                _ => return Err(GlError::BadAnswer("merge requests are not an array".into()).into()),
            };
            let reached = since.is_none_or(|s| items.iter().any(|v| v["updated_at"].as_str().is_some_and(|u| u < s)));
            out.extend(
                items
                    .iter()
                    .filter(|v| since.is_none_or(|s| v["updated_at"].as_str().is_some_and(|u| u >= s)))
                    .filter_map(|v| repo_mr(self, repo, v)),
            );
            if reached || items.len() < PAGE {
                break;
            }
        }
        Ok(out)
    }
    fn change_request_stats(&self, repo: &str, number: u32) -> Result<ChangeStats, ForgeError> {
        let files = self.change_request_files(repo, number)?;
        Ok(ChangeStats {
            changed_files: files.len() as u64,
            additions: files.iter().map(|f| f.additions).sum(),
            deletions: files.iter().map(|f| f.deletions).sum(),
        })
    }
    fn change_request_files(&self, repo: &str, number: u32) -> Result<Vec<ChangeFile>, ForgeError> {
        let diffs = self.get_pages(&format!("{}/diffs?per_page=100", Self::mr_path(repo, number)))?;
        Ok(diffs.iter().filter_map(parse_diff).collect())
    }
    /// An approval, with the body as a comment; GitLab's API can't request changes, so that review (and a plain one)
    /// is a comment.
    fn review_change_request(
        &self,
        repo: &str,
        number: u32,
        verdict: ReviewVerdict,
        body: &str,
    ) -> Result<String, ForgeError> {
        let mr = Self::mr_path(repo, number);
        let body = match verdict {
            ReviewVerdict::Approve => {
                self.send_json("POST", &format!("{mr}/approve"), json!({}))?;
                if body.trim().is_empty() {
                    return Ok(self.mr_url(repo, number));
                }
                body.to_string()
            }
            ReviewVerdict::RequestChanges => format!("**Changes requested**\n\n{body}"),
            ReviewVerdict::Comment => body.to_string(),
        };
        Ok(self.post_note(repo, "merge_requests", number.into(), &body)?)
    }
    fn close_change_request(&self, repo: &str, number: u32) -> Result<(), ForgeError> {
        self.send_json("PUT", &Self::mr_path(repo, number), json!({"state_event": "close"}))?;
        Ok(())
    }
    /// The issue's id is its `iid`: what `/duplicate #<iid>` names.
    fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueDetail>, ForgeError> {
        match self.get(&format!("{}/issues/{number}", project_path(repo))) {
            Ok(v) => {
                let issue = parse_issue(&v).ok_or_else(|| GlError::BadAnswer("no issue iid in the answer".into()))?;
                Ok(Some(IssueDetail {
                    id: issue.number,
                    closed: v["state"] == "closed",
                    state_reason: None,
                    change_request: false,
                    issue,
                }))
            }
            Err(GlError::Status(404, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<ChangeComment>, ForgeError> {
        let notes = self.get_pages(&format!(
            "{}/issues/{number}/notes?sort=asc&order_by=created_at&per_page=100",
            project_path(repo)
        ))?;
        Ok(notes
            .iter()
            .filter(|n| n["system"].as_bool() != Some(true))
            .filter_map(|n| {
                Some(ChangeComment {
                    id: n["id"].as_u64()?,
                    author: s(&n["author"], "username"),
                    is_bot: is_bot(&n["author"]),
                    body: s(n, "body"),
                    url: self.note_url(repo, "issues", number, n),
                    path: None,
                    line: None,
                    in_review: false,
                    collaborator: self.insider(repo, &n["author"]),
                })
            })
            .collect())
    }
    /// GitLab makes labels the project doesn't have yet.
    fn create_issue(&self, repo: &str, new: &NewIssue) -> Result<Issue, ForgeError> {
        let mut body = json!({"title": new.title, "description": comment_body(&new.body)});
        if !new.labels.is_empty() {
            body["labels"] = json!(new.labels.join(","));
        }
        let v = self.send_json("POST", &format!("{}/issues", project_path(repo)), body)?;
        Ok(parse_issue(&v).ok_or_else(|| GlError::BadAnswer("no issue iid in the answer".into()))?)
    }
    fn comment_issue(&self, repo: &str, number: u64, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_note(repo, "issues", number, body)?)
    }
    fn add_labels(&self, repo: &str, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        if labels.is_empty() {
            return Ok(());
        }
        let path = format!("{}/issues/{number}", project_path(repo));
        self.send_json("PUT", &path, json!({"add_labels": labels.join(",")}))?;
        Ok(())
    }
    /// A duplicate is closed by the `/duplicate` quick action (it links the canonical issue); any other reason just
    /// closes it (GitLab keeps no reason).
    fn close_issue(
        &self,
        repo: &str,
        number: u64,
        reason: CloseReason,
        duplicate_of: Option<u64>,
    ) -> Result<(), ForgeError> {
        if let (CloseReason::Duplicate, Some(of)) = (reason, duplicate_of) {
            self.post_note(repo, "issues", number, &format!("/duplicate #{of}"))?;
            return Ok(());
        }
        self.send_json("PUT", &format!("{}/issues/{number}", project_path(repo)), json!({"state_event": "close"}))?;
        Ok(())
    }
    fn release(&self, repo: &str, tag: Option<&str>) -> Result<Option<Release>, ForgeError> {
        let path = match tag {
            Some(t) => format!("{}/releases/{}", project_path(repo), encode(t)),
            None => format!("{}/releases/permalink/latest", project_path(repo)),
        };
        match self.get(&path) {
            Ok(v) => Ok(parse_release(&v)?),
            Err(GlError::Status(404, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    /// GitLab has no pre-releases: the newest release out.
    fn newest_release(&self, repo: &str) -> Result<Option<Release>, ForgeError> {
        let list = self.get(&format!("{}/releases?order_by=released_at&sort=desc&per_page=10", project_path(repo)))?;
        for v in list.as_array().into_iter().flatten() {
            if let Some(r) = parse_release(v)? {
                return Ok(Some(r));
            }
        }
        Ok(None)
    }
}
