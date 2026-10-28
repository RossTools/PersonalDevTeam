//! The forge against recorded GitHub answers ([`Recorded`]): never the network.

use super::*;
use pdt_core::pr::CiState;
use transport::testing::Recorded;

const API: &str = "https://api.github.com";
const GQL: &str = "https://api.github.com/graphql";

fn forge() -> (GithubApi, Arc<Recorded>) {
    let t = Arc::new(Recorded::new());
    let g = GithubApi::new(t.clone());
    g.set_account(Some(Account { token: "gho_t".into(), api_url: API.into(), login: "me".into() }));
    (g, t)
}

fn body_of(r: &Request) -> Value {
    serde_json::from_str(r.body.as_deref().unwrap_or("null")).unwrap()
}

#[test]
fn without_a_login_nothing_is_sent_and_the_inbox_is_asked() {
    let t = Arc::new(Recorded::new());
    let g = GithubApi::new(t.clone());
    assert!(!g.login_needed(), "nothing wanted GitHub yet");
    let e = g.branch_runs("o/r", "b").unwrap_err();
    assert!(matches!(&e, ForgeError::Unavailable(m) if m.contains("GitHub is not connected")), "{e:?}");
    assert!(g.login_needed());
    assert!(t.sent().is_empty());
    g.set_account(Some(Account { token: "x".into(), api_url: API.into(), login: "me".into() }));
    assert!(!g.login_needed());
    assert_eq!(GithubApi::offline().change_request("o/r", "b"), Err(GhError::NotConnected.into()));
}

#[test]
fn calls_carry_the_token_and_a_rejected_one_asks_to_connect_again() {
    let (g, t) = forge();
    t.on("GET", &format!("{API}/user"), Response::new(401, r#"{"message":"Bad credentials"}"#));
    let e = g.viewer_login().unwrap_err();
    assert!(matches!(&e, ForgeError::Unavailable(m) if m.contains("connect GitHub again")), "{e:?}");
    let sent = &t.sent()[0];
    assert!(sent.headers.contains(&("Authorization", "Bearer gho_t".into())));
    assert!(sent.headers.contains(&("X-GitHub-Api-Version", "2022-11-28".into())));
}

#[test]
fn reads_the_branch_pr_preferring_an_open_one() {
    let (g, t) = forge();
    t.ok(
        "POST",
        GQL,
        r#"{"data":{"repository":{"pullRequests":{"nodes":[
      {"number":45,"url":"u45","title":"Old","state":"CLOSED","isDraft":false,"headRefOid":"a","baseRefName":"main",
       "mergeable":"UNKNOWN","mergeStateStatus":"UNKNOWN","isInMergeQueue":false,"mergeQueueEntry":null,"autoMergeRequest":null},
      {"number":44,"url":"u44","title":"New","state":"OPEN","isDraft":true,"headRefOid":"b","baseRefName":"main",
       "mergeable":"CONFLICTING","mergeStateStatus":"BEHIND","isInMergeQueue":true,"mergeQueueEntry":{"position":0},
       "autoMergeRequest":{"enabledAt":"2026-10-18T00:00:00Z"}}]}}}}"#,
    );
    let pr = g.change_request("example-org/PersonalDevTeam", "pdt/PDT-1-x").unwrap().unwrap();
    assert_eq!(pr.number, 44);
    assert_eq!(pr.state, PrState::Open);
    assert!(pr.draft && pr.behind && pr.auto_merge && pr.in_merge_queue);
    assert_eq!(pr.queue_position, Some(1));
    assert_eq!(pr.mergeable, Mergeable::Conflicting);
    assert_eq!((pr.head_sha.as_str(), pr.base_branch.as_str()), ("b", "main"));
    assert_eq!((pr.queue_removal.clone(), pr.created_at), (None, None));
    let vars = &body_of(&t.sent()[0])["variables"];
    assert_eq!(vars, &json!({"owner": "example-org", "name": "PersonalDevTeam", "head": "pdt/PDT-1-x"}));
    assert!(g.change_request("nope", "b").is_err(), "not owner/name");

    t.ok("POST", GQL, r#"{"data":{"repository":{"pullRequests":{"nodes":[]}}}}"#);
    assert_eq!(g.change_request("o/r", "b").unwrap(), None);
    t.ok("POST", GQL, r#"{"data":null,"errors":[{"message":"Could not resolve to a Repository"}]}"#);
    assert!(matches!(g.change_request("o/r", "b"), Err(ForgeError::Failed(m)) if m.contains("Could not resolve")));
}

#[test]
fn pr_nodes_parse_merged_and_queue_removals() {
    let merged = json!({"number":3,"url":"u","title":"t","createdAt":"2026-10-18T00:00:01Z","state":"MERGED",
        "isDraft":false,"headRefOid":"c","baseRefName":"main","mergeable":"UNKNOWN","mergeStateStatus":"UNKNOWN",
        "isInMergeQueue":false,"mergeQueueEntry":null,"autoMergeRequest":null});
    let pr = parse_pr(&merged);
    assert_eq!((pr.state, pr.auto_merge, pr.queue_position), (PrState::Merged, false, None));
    assert_eq!(pr.created_at, pdt_core::conversation::devin::iso_ms("2026-10-18T00:00:01Z"));
    let mut removed = merged.clone();
    removed["state"] = json!("OPEN");
    removed["timelineItems"] = json!({"nodes":[{"__typename":"RemovedFromMergeQueueEvent","id":"RFMQE_x",
        "reason":"merge_conflict","actor":{"login":"github-merge-queue"}}]});
    let r = parse_pr(&removed).queue_removal.unwrap();
    assert_eq!((r.id.as_str(), r.reason.as_str(), r.by_queue), ("RFMQE_x", "merge_conflict", true));
    removed["timelineItems"]["nodes"][0]["actor"]["login"] = json!("someone");
    assert!(!parse_pr(&removed).queue_removal.unwrap().by_queue);
    removed["timelineItems"]["nodes"][0]["__typename"] = json!("AddedToMergeQueueEvent");
    assert_eq!(parse_pr(&removed).queue_removal, None, "queued again since");
}

#[test]
fn reads_a_pr_by_number() {
    let (g, t) = forge();
    t.ok("POST", GQL, r#"{"data":{"repository":{"pullRequest":{"number":7,"state":"OPEN","url":"u7"}}}}"#);
    t.ok(
        "POST",
        GQL,
        r#"{"data":{"repository":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND",
        "message":"Could not resolve to a PullRequest with the number of 8."}]}"#,
    );
    assert_eq!(g.change_request_number("o/r", 7).unwrap().unwrap().url, "u7");
    assert_eq!(body_of(&t.sent()[0])["variables"]["number"], 7);
    assert_eq!(g.change_request_number("o/r", 8).unwrap(), None);
}

#[test]
fn reads_runs_and_failed_jobs_from_actions() {
    let (g, t) = forge();
    t.ok(
        "GET",
        &format!("{API}/repos/o/r/actions/runs?branch=pdt%2Fx&per_page=30"),
        r#"{"total_count":2,"workflow_runs":[
          {"id":391,"run_attempt":2,"name":"ci","status":"completed","conclusion":"failure","head_sha":"abc",
           "head_branch":"pdt/x","html_url":"https://r/391","created_at":"2026-10-19T02:00:10Z"},
          {"id":392,"run_attempt":1,"name":"bench","status":"in_progress","conclusion":null,"head_sha":"abc",
           "head_branch":"pdt/x","html_url":"https://r/392","created_at":"2026-10-19T02:01:10Z"}]}"#,
    );
    let runs = g.branch_runs("o/r", "pdt/x").unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].id, runs[0].attempt, runs[0].state), (391, 2, CiState::Failed));
    assert_eq!(
        (runs[0].workflow.as_str(), runs[0].head_sha.as_str(), runs[0].url.as_str()),
        ("ci", "abc", "https://r/391")
    );
    assert_eq!(runs[1].state, CiState::Running);

    t.ok(
        "GET",
        &format!("{API}/repos/o/r/actions/runs/391/attempts/2/jobs"),
        r#"{"jobs":[{"name":"changes","conclusion":"success"},{"name":"test (linux)","conclusion":"failure"},
          {"name":"release","conclusion":"skipped"}]}"#,
    );
    assert_eq!(g.failed_jobs("o/r", 391, 2).unwrap(), vec!["test (linux)".to_string()]);
    t.ok("GET", &format!("{API}/repos/o/r/actions/runs?event=merge_group"), r#"{"workflow_runs":[]}"#);
    assert!(g.queue_runs("o/r").unwrap().is_empty());
}

#[test]
fn failed_logs_are_the_tails_of_the_failed_jobs() {
    let (g, t) = forge();
    t.ok(
        "GET",
        &format!("{API}/repos/o/r/actions/runs/391/jobs?filter=latest"),
        r#"{"jobs":[{"id":1,"name":"fmt","conclusion":"success","html_url":"j1"},
          {"id":2,"name":"test","conclusion":"failure","html_url":"j2"},
          {"id":3,"name":"old","conclusion":"timed_out","html_url":"j3"}]}"#,
    );
    let log: String = (1..=250).map(|i| format!("2026-10-19T02:00:10.1234567Z line {i}\n")).collect();
    t.ok(
        "GET",
        &format!("{API}/repos/o/r/actions/jobs/2/logs"),
        &format!("{log}2026-10-19T02:00:11.0Z \u{1b}[31merror\u{1b}[0m: boom\n"),
    );
    let logs = g.failed_logs("o/r", 391).unwrap();
    assert_eq!(logs.len(), 2, "the failed and the timed-out job");
    assert_eq!((logs[0].name.as_str(), logs[0].url.as_str()), ("test", "j2"));
    assert!(logs[0].log.starts_with("… (51 earlier lines)\nline 52\n"), "{}", &logs[0].log[..60]);
    assert!(logs[0].log.ends_with("line 250\nerror: boom\n"), "timestamps and colours dropped");
    assert!(logs[1].log.contains("no log"), "job 3's log is gone (404)");
    assert!(!t.sent()[1].headers.iter().any(|(k, _)| *k == "If-None-Match"), "logs are not cached");
}

#[test]
fn unchanged_answers_come_from_the_etag_cache() {
    let (g, t) = forge();
    let url = format!("{API}/repos/o/r/actions/runs?branch=b&per_page=30");
    let body = r#"{"workflow_runs":[{"id":1,"run_attempt":1,"name":"ci","status":"queued","conclusion":null}]}"#;
    t.on("GET", &url, Response::new(200, body).with_header("ETag", "W/\"e1\""));
    t.on("GET", &url, Response::new(304, ""));
    assert_eq!(g.branch_runs("o/r", "b").unwrap().len(), 1);
    assert_eq!(g.branch_runs("o/r", "b").unwrap().len(), 1, "a 304 gives the cached answer");
    let second = &t.sent()[1];
    assert!(second.headers.contains(&("If-None-Match", "W/\"e1\"".into())));
}

#[test]
fn a_used_up_rate_limit_stops_calls_until_it_resets() {
    let (g, t) = forge();
    let reset = now_secs() + 600;
    t.on(
        "GET",
        &format!("{API}/user"),
        Response::new(403, r#"{"message":"API rate limit exceeded"}"#)
            .with_header("x-ratelimit-remaining", "0")
            .with_header("x-ratelimit-reset", &reset.to_string()),
    );
    let e = g.viewer_login().unwrap_err();
    assert!(matches!(&e, ForgeError::Failed(m) if m.contains("rate limit is used up until")), "{e:?}");
    assert!(g.branch_runs("o/r", "b").is_err());
    assert_eq!(t.sent().len(), 1, "nothing is sent while the limit is used up");
    let (g, t) = forge();
    t.on("GET", &format!("{API}/user"), Response::new(429, "{}").with_header("retry-after", "30"));
    assert!(matches!(g.viewer_login(), Err(ForgeError::Failed(m)) if m.contains("rate limit")));
    assert_eq!(
        next_link(r#"<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last""#),
        Some("https://api.github.com/x?page=2".into())
    );
    assert_eq!(next_link(r#"<https://api.github.com/x?page=1>; rel="prev""#), None);
}

#[test]
fn comments_and_reviews_follow_their_pages() {
    let (g, t) = forge();
    let c = format!("{API}/repos/o/r/issues/3/comments?per_page=100");
    t.on(
        "GET",
        &c,
        Response::new(200, r#"[{"id":1,"user":{"login":"al","type":"User"},"body":"hi","html_url":"u1","author_association":"MEMBER"}]"#)
            .with_header("Link", &format!("<{API}/repositories/9/issues/3/comments?page=2>; rel=\"next\"")),
    );
    t.ok(
        "GET",
        &format!("{API}/repositories/9/issues/3/comments?page=2"),
        r#"[{"id":2,"user":{"login":"ci[bot]","type":"Bot"},"body":"green","html_url":"u2"}]"#,
    );
    t.ok(
        "GET",
        &format!("{API}/repos/o/r/pulls/3/comments"),
        r#"[{"id":11,"user":{"login":"x","type":"User"},"body":"nit","html_url":"u","path":"src/a.rs","line":40,
            "author_association":"CONTRIBUTOR"}]"#,
    );
    let all = g.change_comments("o/r", 3).unwrap();
    assert_eq!(all.iter().map(|c| c.id).collect::<Vec<_>>(), [1, 2, 11]);
    assert!(!all[0].is_bot && all[1].is_bot);
    assert_eq!((all[2].path.as_deref(), all[2].line, all[2].in_review), (Some("src/a.rs"), Some(40), true));
    // PDT-202: only the owner, members and collaborators are insiders (a contributor or no association is not).
    assert_eq!(all.iter().map(|c| c.collaborator).collect::<Vec<_>>(), [true, false, false]);

    t.ok(
        "GET",
        &format!("{API}/repos/o/r/pulls/3/reviews"),
        r#"[{"id":5,"user":{"login":"al"},"state":"CHANGES_REQUESTED","body":"no","html_url":"u","author_association":"OWNER"},
           {"id":6,"user":{"login":"al"},"state":"PENDING","body":"","html_url":"u"}]"#,
    );
    let r = g.change_reviews("o/r", 3).unwrap();
    assert_eq!((r.len(), r[0].state.as_str(), r[0].collaborator), (1, "changes_requested", true));
}

#[test]
fn comments_and_replies_on_a_pr() {
    let (g, t) = forge();
    t.ok(
        "POST",
        &format!("{API}/repos/o/r/issues/3/comments"),
        r#"{"html_url":"https://github.com/o/r/pull/3#issuecomment-1"}"#,
    );
    t.ok(
        "POST",
        &format!("{API}/repos/o/r/pulls/3/comments/11/replies"),
        r#"{"html_url":"https://github.com/o/r/pull/3#discussion_r2"}"#,
    );
    assert!(g.comment_change_request("o/r", 3, "done").unwrap().ends_with("issuecomment-1"));
    assert!(g.reply_review_comment("o/r", 3, 11, "fixed").unwrap().ends_with("discussion_r2"));
    assert_eq!(body_of(&t.sent()[0]), json!({"body": "done"}));
    assert_eq!(body_of(&t.sent()[1]), json!({"body": "fixed"}));
    t.ok("POST", &format!("{API}/repos/o/r/issues/3/comments"), "{}");
    g.comment_change_request("o/r", 3, &"x".repeat(MAX_COMMENT + 5)).unwrap();
    assert_eq!(body_of(&t.sent()[2])["body"].as_str().unwrap().chars().count(), MAX_COMMENT, "cut to the limit");
}

#[test]
fn opens_a_pull_request_and_names_an_unpushed_head() {
    let (g, t) = forge();
    let url = format!("{API}/repos/o/r/pulls");
    t.ok("POST", &url, r#"{"number":42,"html_url":"https://github.com/o/r/pull/42"}"#);
    let new = NewChangeRequest {
        head: "pdt/x".into(),
        base: "main".into(),
        title: "Fix it".into(),
        body: "Why".into(),
        draft: true,
    };
    let opened = g.open_change_request("o/r", &new).unwrap();
    assert_eq!(opened, OpenedChangeRequest { number: 42, url: "https://github.com/o/r/pull/42".into() });
    assert_eq!(
        body_of(&t.sent()[0]),
        json!({"title": "Fix it", "head": "pdt/x", "base": "main", "body": "Why", "draft": true})
    );
    t.on(
        "POST",
        &url,
        Response::new(
            422,
            r#"{"message":"Validation Failed","errors":[{"resource":"PullRequest","code":"invalid","field":"head"}]}"#,
        ),
    );
    let e = g.open_change_request("o/r", &new).unwrap_err().to_string();
    assert!(e.contains("Validation Failed (invalid)") && e.contains("Is pdt/x pushed"), "{e}");
}

#[test]
fn issue_pages_are_read_back_to_the_last_issue_seen() {
    // 250 issues and PRs, newest first, 100 a page.
    let page = |n: u32| -> Result<Vec<Value>, GhError> {
        let top = 250 - u64::from(n - 1) * 100;
        Ok((top.saturating_sub(99).max(1)..=top)
            .rev()
            .map(|i| if i % 10 == 0 { json!({"number": i, "pull_request": {}}) } else { json!({"number": i}) })
            .collect())
    };
    let mut asked = vec![];
    let first = read_issue_pages(None, |n| {
        asked.push(n);
        page(n)
    })
    .unwrap();
    assert_eq!((asked.as_slice(), first.len()), (&[1][..], 90), "the first look reads one page, PRs left out");
    asked.clear();
    let since = read_issue_pages(Some(120), |n| {
        asked.push(n);
        page(n)
    })
    .unwrap();
    assert_eq!(asked, [1, 2], "page 2 reaches #120");
    assert!(since.iter().any(|i| i.number == 121) && since.iter().all(|i| i.number % 10 != 0));
    asked.clear();
    read_issue_pages(Some(0), |n| {
        asked.push(n);
        page(n)
    })
    .unwrap();
    assert_eq!(asked, [1, 2, 3], "the third page is the last");
}

#[test]
fn issues_are_read_commented_labelled_and_closed() {
    let (g, t) = forge();
    let one = r#"{"id":901,"number":12,"title":"Crash","body":null,"html_url":"https://github.com/o/r/issues/12",
      "user":{"login":"al"},"labels":[{"name":"from-pdt"}],"created_at":"t","state":"closed","state_reason":"not_planned"}"#;
    t.ok("GET", &format!("{API}/repos/o/r/issues/12"), one);
    let i = g.issue("o/r", 12).unwrap().unwrap();
    assert_eq!((i.id, i.issue.number, i.closed, i.state_reason.as_deref()), (901, 12, true, Some("not_planned")));
    assert_eq!(
        (i.issue.body.as_str(), i.issue.labels.clone(), i.issue.author.as_str()),
        ("", vec!["from-pdt".to_string()], "al")
    );
    assert!(!i.change_request);
    assert_eq!(g.issue("o/r", 13).unwrap(), None, "a 404");
    assert!(parse_issue(&json!({})).is_err());

    t.ok(
        "GET",
        &format!("{API}/repos/o/r/issues?state=all&sort=created&direction=desc&per_page=100&page=1"),
        &format!(r#"[{one}, {{"number":11,"pull_request":{{"url":"x"}}}}]"#),
    );
    let list = g.recent_issues("o/r", None).unwrap();
    assert_eq!(list.iter().map(|i| i.number).collect::<Vec<_>>(), [12], "pull requests are left out");

    t.ok(
        "GET",
        &format!("{API}/repos/o/r/issues/12/comments"),
        r#"[{"id":4,"user":{"login":"al"},"body":"same here"}]"#,
    );
    assert_eq!(g.issue_comments("o/r", 12).unwrap()[0].body, "same here");

    t.ok("POST", &format!("{API}/repos/o/r/issues/12/comments"), r#"{"html_url":"https://c/1"}"#);
    assert_eq!(g.comment_issue("o/r", 12, "@here: hi").unwrap(), "https://c/1");

    t.ok("POST", &format!("{API}/repos/o/r/issues/12/labels"), "[]");
    g.add_labels("o/r", 12, &["bug".into(), "p1".into()]).unwrap();
    g.add_labels("o/r", 12, &[]).unwrap();

    t.ok("PATCH", &format!("{API}/repos/o/r/issues/12"), "{}");
    t.ok("PATCH", &format!("{API}/repos/o/r/issues/12"), "{}");
    g.close_issue("o/r", 12, CloseReason::Duplicate, Some(901)).unwrap();
    g.close_issue("o/r", 12, CloseReason::NotPlanned, Some(901)).unwrap();
    let sent = t.sent();
    let n = sent.len();
    assert_eq!(
        body_of(&sent[n - 3]),
        json!({"labels": ["bug", "p1"]}),
        "added (POST), not replaced; none sends nothing"
    );
    assert_eq!(
        body_of(&sent[n - 2]),
        json!({"state": "closed", "state_reason": "duplicate", "duplicate_issue_id": 901})
    );
    assert_eq!(body_of(&sent[n - 1]), json!({"state": "closed", "state_reason": "not_planned"}));
}

#[test]
fn merges_turn_on_auto_merge_or_merge_a_clean_pr() {
    let (g, t) = forge();
    let id = r#"{"data":{"repository":{"pullRequest":{"id":"PR_7"}}}}"#;
    t.ok("POST", GQL, id).ok("POST", GQL, r#"{"data":{"enablePullRequestAutoMerge":{"clientMutationId":null}}}"#);
    g.merge("o/r", 7, Some(MergeStrategy::Squash)).unwrap();
    assert_eq!(body_of(&t.sent()[1])["variables"], json!({"id": "PR_7", "method": "SQUASH"}));

    let clean = r#"{"errors":[{"message":"Pull request Pull request is in clean status"}]}"#;
    t.ok("POST", GQL, id).ok("POST", GQL, clean);
    t.ok("PUT", &format!("{API}/repos/o/r/pulls/7/merge"), r#"{"merged":true}"#);
    g.merge("o/r", 7, Some(MergeStrategy::Rebase)).unwrap();
    let sent = t.sent();
    assert_eq!(sent.last().unwrap().method, "PUT");
    assert_eq!(body_of(sent.last().unwrap()), json!({"merge_method": "rebase"}));

    // A merge queue picks the strategy: a clean PR joins the queue.
    t.ok("POST", GQL, id).ok("POST", GQL, clean).ok(
        "POST",
        GQL,
        r#"{"data":{"enqueuePullRequest":{"clientMutationId":null}}}"#,
    );
    g.merge("o/r", 7, None).unwrap();
    let sent = t.sent();
    assert_eq!(body_of(&sent[sent.len() - 2])["variables"]["method"], Value::Null);
    assert!(body_of(sent.last().unwrap())["query"].as_str().unwrap().contains("enqueuePullRequest"));

    t.ok("POST", GQL, id).ok("POST", GQL, r#"{"errors":[{"message":"Auto merge is not allowed"}]}"#);
    assert!(matches!(g.merge("o/r", 7, None), Err(ForgeError::Failed(m)) if m.contains("not allowed")));
}

#[test]
fn reads_merge_settings() {
    let (g, t) = forge();
    t.ok(
        "POST",
        GQL,
        r#"{"data":{"repository":{"squashMergeAllowed":true,"mergeCommitAllowed":false,
      "rebaseMergeAllowed":true,"mergeQueue":{"id":"MQ_1"}}}}"#,
    );
    assert_eq!(
        g.merge_settings("o/r", "main").unwrap(),
        MergeSettings { squash: true, merge: false, rebase: true, queue: true }
    );
    assert_eq!(body_of(&t.sent()[0])["variables"]["base"], "main");
    assert!(!parse_merge_settings(&json!({"repository":{"mergeQueue":null}})).unwrap().queue);
    assert!(parse_merge_settings(&json!({})).is_err());
}

#[test]
fn enterprise_calls_go_to_its_api() {
    let t = Arc::new(Recorded::new());
    let g = GithubApi::new(t.clone());
    g.set_account(Some(Account {
        token: "t".into(),
        api_url: "https://ghe.example.com/api/v3".into(),
        login: "me".into(),
    }));
    let _ = g.viewer_login();
    let _ = g.change_request("o/r", "b");
    assert_eq!(t.urls(), ["GET https://ghe.example.com/api/v3/user", "POST https://ghe.example.com/api/graphql"]);
}

/// What agents on ticket work are told about GitHub: PDT's tools, never `gh`.
#[test]
fn agents_read_ci_and_answer_with_pdt_tools() {
    use pdt_core::agent_context::{SessionContext, TicketWork, session_instructions};
    use pdt_core::workflow::events as ev;
    let notes = agent_notes();
    let news = [ev::CI_RUN_FAILED, ev::PR_COMMENTED].into_iter().collect();
    let ctx = SessionContext {
        port: 7420,
        session_id: "s1",
        project_id: "p1",
        project_name: "Work App",
        project_key: "WA",
        checkout: "/r/.worktrees/x",
        branch: Some("pdt-WA-7-x"),
        repo_path: "/r",
        base_branch: "main",
        ticket: Some(TicketWork { ticket_id: "t7", key: "WA-7", branch: "pdt-WA-7-x", base: "main", news }),
        personal: false,
        tools: &[],
        step: None,
        ci_branch: Some("pdt-WA-7-x"),
        forge: Some(&notes),
    };
    let t = session_instructions(&ctx, None);
    assert!(t.contains("`ci_runs` (the runs on pdt-WA-7-x and their failed jobs)"), "{t}");
    assert!(t.contains("`ci_failed_logs`") && t.contains("`change_request_view`"), "{t}");
    assert!(t.contains("answer it on the pull request (`change_request_comment`, with `replyTo`"), "{t}");
    assert!(t.contains("watch CI in a shell (sleep loops)"), "{t}");
    assert!(!t.contains("`gh "), "no gh commands: {t}");
    assert_eq!(GithubApi::offline().agent_notes(), notes);
    assert_eq!(GithubApi::offline().capabilities(), Capabilities::ALL);
}
