//! The GitHub forge: [`Forge`] for [`GithubApi`], one method per forge operation.

use super::*;

impl Forge for GithubApi {
    fn name(&self) -> &'static str {
        "GitHub"
    }
    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }
    fn agent_notes(&self) -> ForgeNotes {
        agent_notes()
    }
    fn queue_ref(&self, base: &str, number: u32) -> Option<String> {
        Some(queue_branch_prefix(base, number))
    }
    fn login_needed(&self) -> bool {
        (self.account().is_none() && self.wanted.load(Ordering::SeqCst)) || self.rejected_at().is_some()
    }
    fn login_rejected_at(&self) -> Option<i64> {
        self.rejected_at()
    }
    fn change_request(&self, repo: &str, branch: &str) -> Result<Option<ChangeRequest>, ForgeError> {
        let (owner, name) = owner_name(repo)?;
        let d = self.graphql(&pr_query(), json!({"owner": owner, "name": name, "head": branch}))?;
        Ok(parse_pull_requests(&d)?)
    }
    fn change_request_number(&self, repo: &str, number: u32) -> Result<Option<ChangeRequest>, ForgeError> {
        let (owner, name) = owner_name(repo)?;
        match self.graphql(&pr_number_query(), json!({"owner": owner, "name": name, "number": number})) {
            Ok(d) => Ok(Some(&d["repository"]["pullRequest"]).filter(|n| n.is_object()).map(parse_pr)),
            Err(GhError::Status(200, m)) if m.contains("Could not resolve to a PullRequest") => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn open_change_request(&self, repo: &str, new: &NewChangeRequest) -> Result<OpenedChangeRequest, ForgeError> {
        let body = json!({
            "title": new.title,
            "head": new.head,
            "base": new.base,
            "body": comment_body(&new.body),
            "draft": new.draft,
        });
        match self.send_json("POST", &format!("repos/{repo}/pulls"), body) {
            Ok(v) => Ok(parse_opened(&v)?),
            // A head GitHub doesn't have, or one with nothing the base lacks.
            Err(GhError::Status(422, m)) => Err(ForgeError::Failed(format!(
                "GitHub refused the pull request: {m}. Is {} pushed, with commits {} doesn't have?",
                new.head, new.base
            ))),
            Err(e) => Err(e.into()),
        }
    }
    fn merge_settings(&self, repo: &str, base: &str) -> Result<MergeSettings, ForgeError> {
        let (owner, name) = owner_name(repo)?;
        Ok(parse_merge_settings(
            &self.graphql(MERGE_SETTINGS_QUERY, json!({"owner": owner, "name": name, "base": base}))?,
        )?)
    }
    fn merge(&self, repo: &str, number: u32, strategy: Option<MergeStrategy>) -> Result<(), ForgeError> {
        self.merge_head(repo, number, strategy, None)
    }
    /// GraphQL's `expectedHeadOid`, and REST's `sha` for a merge at once. Auto-merge where the repository allows it;
    /// otherwise (PDT-297), and for a pull request that could merge now, into the merge queue (no strategy) or merged
    /// at once. A repository without auto-merge can't hold a merge for its checks: PDT's callers wait for CI first.
    fn merge_head(
        &self,
        repo: &str,
        number: u32,
        strategy: Option<MergeStrategy>,
        head: Option<&str>,
    ) -> Result<(), ForgeError> {
        let (id, auto_merge) = self.pr_node_id(repo, number)?;
        let method = strategy.map(merge_method);
        let (auto, enqueue, vars, queue_vars) = match head {
            Some(h) => (
                AUTO_MERGE_AT,
                ENQUEUE_AT,
                json!({"id": id, "method": method, "head": h}),
                json!({"id": id, "head": h}),
            ),
            None => (AUTO_MERGE, ENQUEUE, json!({"id": id, "method": method}), json!({"id": id})),
        };
        if auto_merge {
            match self.graphql(auto, vars) {
                Ok(_) => return Ok(()),
                // GitHub refuses auto-merge for a PR that could merge now ("is in clean status"), and where the
                // repository turned it off since: merge it as a repository without auto-merge does.
                Err(GhError::Status(200, m))
                    if m.contains("clean status") || m.contains("Auto merge is not allowed") => {}
                // Checks that aren't required still running (or failing): auto-merge can't wait for them.
                Err(GhError::Status(200, m)) if m.contains("unstable status") => {
                    return Err(ForgeError::NotReady(format!(
                        "GitHub can't hold the merge of #{number} for checks {repo} doesn't require"
                    )));
                }
                Err(e) => return Err(e.into()),
            }
        }
        // Through the merge queue when no strategy is given (a queue picks it), else at once.
        if strategy.is_none() && self.graphql(enqueue, queue_vars).is_ok() {
            return Ok(());
        }
        Ok(self.merge_now(repo, number, strategy, head)?)
    }
    fn viewer_login(&self) -> Result<String, ForgeError> {
        let login = s(&self.get("user")?, "login");
        if login.is_empty() { Err(GhError::BadAnswer("no login".into()).into()) } else { Ok(login) }
    }
    fn recent_issues(&self, repo: &str, after: Option<u64>) -> Result<Vec<Issue>, ForgeError> {
        Ok(read_issue_pages(after, |n| {
            let path =
                format!("repos/{repo}/issues?state=all&sort=created&direction=desc&per_page={ISSUE_PAGE}&page={n}");
            match self.get(&path)? {
                Value::Array(items) => Ok(items),
                _ => Err(GhError::BadAnswer("issues are not an array".into())),
            }
        })?)
    }
    fn recent_change_requests(&self, repo: &str, since: Option<&str>) -> Result<Vec<RepoChangeRequest>, ForgeError> {
        Ok(read_pull_pages(since, |n| {
            let path =
                format!("repos/{repo}/pulls?state=all&sort=updated&direction=desc&per_page={ISSUE_PAGE}&page={n}");
            match self.get(&path)? {
                Value::Array(items) => Ok(items),
                _ => Err(GhError::BadAnswer("pull requests are not an array".into())),
            }
        })?)
    }
    fn change_request_stats(&self, repo: &str, number: u32) -> Result<ChangeStats, ForgeError> {
        let v = self.get(&format!("repos/{repo}/pulls/{number}"))?;
        let n = |k: &str| v[k].as_u64().unwrap_or(0);
        Ok(ChangeStats { changed_files: n("changed_files"), additions: n("additions"), deletions: n("deletions") })
    }
    fn change_comments(&self, repo: &str, number: u32) -> Result<Vec<ChangeComment>, ForgeError> {
        let mut all =
            parse_comments(&self.get_pages(&format!("repos/{repo}/issues/{number}/comments?per_page=100"))?, false);
        all.extend(parse_comments(
            &self.get_pages(&format!("repos/{repo}/pulls/{number}/comments?per_page=100"))?,
            true,
        ));
        Ok(all)
    }
    fn change_reviews(&self, repo: &str, number: u32) -> Result<Vec<ChangeReview>, ForgeError> {
        Ok(parse_reviews(&self.get_pages(&format!("repos/{repo}/pulls/{number}/reviews?per_page=100"))?))
    }
    fn comment_change_request(&self, repo: &str, number: u32, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_comment(&format!("repos/{repo}/issues/{number}/comments"), body)?)
    }
    fn reply_review_comment(&self, repo: &str, number: u32, comment_id: u64, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_comment(&format!("repos/{repo}/pulls/{number}/comments/{comment_id}/replies"), body)?)
    }
    /// GitHub lists at most 3000 files, 100 a page ([`MAX_PAGES`] pages are read).
    fn change_request_files(&self, repo: &str, number: u32) -> Result<Vec<ChangeFile>, ForgeError> {
        Ok(parse_files(&self.get_pages(&format!("repos/{repo}/pulls/{number}/files?per_page=100"))?))
    }
    fn review_change_request(
        &self,
        repo: &str,
        number: u32,
        verdict: ReviewVerdict,
        body: &str,
    ) -> Result<String, ForgeError> {
        let body = json!({"event": review_event(verdict), "body": comment_body(body)});
        let v = self.send_json("POST", &format!("repos/{repo}/pulls/{number}/reviews"), body)?;
        Ok(s(&v, "html_url"))
    }
    fn close_change_request(&self, repo: &str, number: u32) -> Result<(), ForgeError> {
        self.send_json("PATCH", &format!("repos/{repo}/pulls/{number}"), json!({"state": "closed"}))?;
        Ok(())
    }
    fn issue(&self, repo: &str, number: u64) -> Result<Option<IssueDetail>, ForgeError> {
        match self.get(&format!("repos/{repo}/issues/{number}")) {
            Ok(v) => Ok(Some(parse_issue(&v)?)),
            Err(GhError::Status(404 | 410, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn issue_comments(&self, repo: &str, number: u64) -> Result<Vec<ChangeComment>, ForgeError> {
        Ok(parse_comments(&self.get_pages(&format!("repos/{repo}/issues/{number}/comments?per_page=100"))?, false))
    }
    /// Labels the login can't set (no push access) or the repository lacks are dropped by GitHub, not refused.
    fn create_issue(&self, repo: &str, new: &NewIssue) -> Result<Issue, ForgeError> {
        let mut body = json!({"title": new.title, "body": comment_body(&new.body)});
        if !new.labels.is_empty() {
            body["labels"] = json!(new.labels);
        }
        Ok(parse_issue(&self.send_json("POST", &format!("repos/{repo}/issues"), body)?)?.issue)
    }
    fn comment_issue(&self, repo: &str, number: u64, body: &str) -> Result<String, ForgeError> {
        Ok(self.post_comment(&format!("repos/{repo}/issues/{number}/comments"), body)?)
    }
    /// A POST adds them to the issue's (a PATCH of `labels` would replace the set).
    fn add_labels(&self, repo: &str, number: u64, labels: &[String]) -> Result<(), ForgeError> {
        if labels.is_empty() {
            return Ok(());
        }
        self.send_json("POST", &format!("repos/{repo}/issues/{number}/labels"), json!({"labels": labels}))?;
        Ok(())
    }
    /// `state_reason`, and `duplicate_issue_id` for a duplicate (GitHub's "Close as duplicate").
    fn close_issue(
        &self,
        repo: &str,
        number: u64,
        reason: CloseReason,
        duplicate_of: Option<u64>,
    ) -> Result<(), ForgeError> {
        let mut body = json!({"state": "closed", "state_reason": reason.as_str()});
        if let (CloseReason::Duplicate, Some(id)) = (reason, duplicate_of) {
            body["duplicate_issue_id"] = json!(id);
        }
        self.send_json("PATCH", &format!("repos/{repo}/issues/{number}"), body)?;
        Ok(())
    }
    fn release(&self, repo: &str, tag: Option<&str>) -> Result<Option<Release>, ForgeError> {
        let path = match tag {
            Some(t) => format!("repos/{repo}/releases/tags/{}", encode(t)),
            None => format!("repos/{repo}/releases/latest"),
        };
        match self.get(&path) {
            Ok(v) => Ok(parse_release(&v)?),
            Err(GhError::Status(404, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    /// GitHub lists releases newest first; drafts are skipped.
    fn newest_release(&self, repo: &str) -> Result<Option<Release>, ForgeError> {
        let list = self.get(&format!("repos/{repo}/releases?per_page=10"))?;
        for v in list.as_array().into_iter().flatten() {
            if let Some(r) = parse_release_or_pre(v, true)? {
                return Ok(Some(r));
            }
        }
        Ok(None)
    }
    fn recent_releases(&self, repo: &str) -> Result<Vec<Release>, ForgeError> {
        let list = self.get(&format!("repos/{repo}/releases?per_page=30"))?;
        let mut out = vec![];
        for v in list.as_array().into_iter().flatten() {
            out.extend(parse_release_or_pre(v, true)?);
        }
        Ok(out)
    }
    /// GitHub's comparison of `base...head` (one commit asked for: only the count is read).
    fn commits_ahead(&self, repo: &str, base: &str, head: &str) -> Result<Option<u64>, ForgeError> {
        match self.get(&format!("repos/{repo}/compare/{}...{}?per_page=1", encode(base), encode(head))) {
            Ok(v) => Ok(Some(
                v["ahead_by"].as_u64().ok_or_else(|| GhError::BadAnswer("a comparison without ahead_by".into()))?,
            )),
            Err(GhError::Status(404, _)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
