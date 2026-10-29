//! The forge against recorded GitHub answers ([`Recorded`]): never the network.

use super::*;
use pdt_core::pr::CiState;
use pdt_forge::CiProvider;
use std::collections::BTreeMap;
use transport::testing::Recorded;

/// The Actions calls of a forge's login, as tests make them.
trait Actions {
    fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError>;
}

impl Actions for Arc<GithubApi> {
    fn branch_runs(&self, repo: &str, branch: &str) -> Result<Vec<CiRun>, ForgeError> {
        GithubActions::new(self.clone()).runs(repo, branch, None)
    }
}

const API: &str = "https://api.github.com";
const GQL: &str = "https://api.github.com/graphql";

fn forge() -> (Arc<GithubApi>, Arc<Recorded>) {
    let t = Arc::new(Recorded::new());
    let g = Arc::new(GithubApi::new(t.clone()));
    g.set_account(Some(Account { token: "gho_t".into(), api_url: API.into(), login: "me".into(), signed_in_at: None }));
    (g, t)
}

fn body_of(r: &Request) -> Value {
    serde_json::from_str(r.body.as_deref().unwrap_or("null")).unwrap()
}

#[test]
fn without_a_login_nothing_is_sent_and_the_inbox_is_asked() {
    let t = Arc::new(Recorded::new());
    let g = Arc::new(GithubApi::new(t.clone()));
    assert!(!g.login_needed(), "nothing wanted GitHub yet");
    let e = g.branch_runs("o/r", "b").unwrap_err();
    assert!(matches!(&e, ForgeError::Unavailable(m) if m.contains("GitHub is not connected")), "{e:?}");
    assert!(g.login_needed());
    assert!(t.sent().is_empty());
    g.set_account(Some(Account { token: "x".into(), api_url: API.into(), login: "me".into(), signed_in_at: None }));
    assert!(!g.login_needed());
    assert_eq!(GithubApi::offline().change_request("o/r", "b"), Err(GhError::NotConnected.into()));
}

#[test]
fn calls_carry_the_token_and_a_rejected_one_asks_to_connect_again() {
    let (g, t) = forge();
    t.on("GET", &format!("{API}/user"), Response::new(401, r#"{"message":"Bad credentials"}"#));
    t.on("GET", &format!("{API}/user"), Response::new(401, r#"{"message":"Bad credentials"}"#));
    let e = g.viewer_login().unwrap_err();
    assert!(matches!(&e, ForgeError::Unavailable(m) if m.contains("connect GitHub again")), "{e:?}");
    let sent = &t.sent()[0];
    assert!(sent.headers.contains(&("Authorization", "Bearer gho_t".into())));
    assert!(sent.headers.contains(&("X-GitHub-Api-Version", "2022-11-28".into())));
}

#[test]
fn a_rejected_login_is_kept_and_nothing_more_is_sent_until_the_user_signs_in_again() {
    let (g, t) = forge();
    assert_eq!((g.login_needed(), g.login_rejected_at()), (false, None));
    t.on("POST", GQL, Response::new(401, r#"{"message":"Bad credentials"}"#));
    t.on("GET", &format!("{API}/user"), Response::new(401, r#"{"message":"Bad credentials"}"#));
    let e = g.change_request("o/r", "b").unwrap_err();
    assert!(matches!(&e, ForgeError::Unavailable(m) if m.contains("connect GitHub again")), "{e:?}");
    assert_eq!(t.urls(), [format!("POST {GQL}"), format!("GET {API}/user")], "GET /user made sure");
    let at = g.login_rejected_at().expect("marked rejected");
    assert!(g.login_needed(), "the Inbox asks to reconnect");
    assert_eq!(g.account().unwrap().login, "me", "the login is kept");

    // The pollers back off: every call fails the same way without reaching GitHub.
    assert!(matches!(g.branch_runs("o/r", "b"), Err(ForgeError::Unavailable(_))));
    assert!(matches!(g.change_request("o/r", "b"), Err(ForgeError::Unavailable(_))));
    assert_eq!(t.sent().len(), 2, "no more calls reached GitHub");
    assert_eq!(g.login_rejected_at(), Some(at), "the first refusal's time");

    // Signing in again clears it, and calls go out again.
    g.set_account(Some(Account {
        token: "gho_new".into(),
        api_url: API.into(),
        login: "me".into(),
        signed_in_at: None,
    }));
    assert_eq!((g.login_needed(), g.login_rejected_at()), (false, None));
    t.ok("GET", &format!("{API}/repos/o/r/actions/runs?branch=b&per_page={RUN_LIMIT}"), r#"{"workflow_runs":[]}"#);
    assert_eq!(g.branch_runs("o/r", "b").unwrap(), vec![]);
    assert_eq!(t.sent().len(), 3);
}

#[test]
fn a_401_github_takes_back_when_asked_again_does_not_mark_the_login() {
    let (g, t) = forge();
    t.on("POST", GQL, Response::new(401, r#"{"message":"Bad credentials"}"#));
    t.ok("GET", &format!("{API}/user"), r#"{"login":"me"}"#);
    let e = g.change_request("o/r", "b").unwrap_err();
    assert!(matches!(&e, ForgeError::Failed(m) if m.contains("401") && m.contains("Bad credentials")), "{e:?}");
    assert_eq!((g.login_needed(), g.login_rejected_at()), (false, None));
}

#[test]
fn a_401_from_a_redirect_to_another_host_does_not_mark_the_login() {
    let (g, t) = forge();
    let mut storage = Response::new(401, "<Error>no auth</Error>");
    storage.url = "https://results-receiver.actions.githubusercontent.com/logs/3".into();
    t.on("GET", &format!("{API}/repos/o/r/actions/jobs/3/logs"), storage);
    let job = pdt_core::pr::CiJob { id: "3".into(), name: "test".into(), state: CiState::Failed, url: "h".into() };
    let run = GithubActions::new(g.clone());
    let r = run.runs("o/r", "b", None);
    assert!(r.is_err(), "nothing recorded for it");
    let some = CiRun {
        provider: String::new(),
        id: "7".into(),
        attempt: 1,
        name: String::new(),
        state: CiState::Failed,
        url: String::new(),
        head_sha: String::new(),
        branch: String::new(),
        created_at: String::new(),
        failed_jobs: vec![],
    };
    assert!(run.job_log_tail("o/r", &some, &job, 10).is_err());
    assert_eq!(g.login_rejected_at(), None);
    assert!(!t.urls().iter().any(|u| u.ends_with("/user")), "nothing asked again: {:?}", t.urls());
}

#[test]
fn the_last_calls_are_kept_without_queries() {
    let (g, t) = forge();
    t.ok("GET", &format!("{API}/repos/o/r/actions/runs"), r#"{"workflow_runs":[]}"#);
    let _ = g.branch_runs("o/r", "b");
    let _ = g.viewer_login();
    let calls: Vec<_> = g.recent_calls().into_iter().map(|c| (c.method, c.path, c.status)).collect();
    assert_eq!(
        calls,
        [("GET", "/repos/o/r/actions/runs".to_string(), Some(200)), ("GET", "/user".to_string(), Some(404))]
    );
    for _ in 0..RECENT_CALLS {
        let _ = g.viewer_login();
    }
    assert_eq!(g.recent_calls().len(), RECENT_CALLS);
}

#[test]
fn the_login_check_finds_a_refused_login_and_clears_the_mark_once_github_accepts_it_again() {
    let (g, t) = forge();
    assert_eq!(GithubApi::new(t.clone()).check_login(), None, "no login, no check");
    t.ok("GET", &format!("{API}/rate_limit"), r#"{"resources":{}}"#);
    assert_eq!(g.check_login(), Some(LoginCheck::Ok));

    t.on("GET", &format!("{API}/rate_limit"), Response::new(401, r#"{"message":"Bad credentials"}"#));
    t.on("GET", &format!("{API}/user"), Response::new(401, r#"{"message":"Bad credentials"}"#));
    assert_eq!(g.check_login(), Some(LoginCheck::Rejected));
    assert!(g.login_rejected_at().is_some());

    // While marked, the check asks `GET /user`.
    t.on("GET", &format!("{API}/user"), Response::new(401, r#"{"message":"Bad credentials"}"#));
    assert_eq!(g.check_login(), Some(LoginCheck::Rejected));
    assert!(g.login_rejected_at().is_some());
    t.ok("GET", &format!("{API}/user"), r#"{"login":"me"}"#);
    assert_eq!(g.check_login(), Some(LoginCheck::Recovered));
    assert_eq!((g.login_needed(), g.login_rejected_at()), (false, None));
    let paths: Vec<_> = t.urls().into_iter().map(|u| u.replace(API, "")).collect();
    assert_eq!(paths, ["GET /rate_limit", "GET /rate_limit", "GET /user", "GET /user", "GET /user"]);
}

#[test]
fn urls_for_logs() {
    assert_eq!(path_of("https://api.github.com/repos/o/r?x=1"), "/repos/o/r");
    assert_eq!(path_of("https://api.github.com"), "/");
    assert!(same_host("", "https://api.github.com/x"));
    assert!(same_host("https://API.github.com/y", "https://api.github.com/x"));
    assert!(!same_host("https://blob.core.windows.net/y", "https://api.github.com/x"));
    assert_eq!(clock_ms(90_061_000), "01:01:01 UTC");
}

#[test]
fn a_stored_login_without_a_sign_in_time_still_reads() {
    let a: Account = serde_json::from_str(r#"{"token":"t","apiUrl":"u","login":"me"}"#).unwrap();
    assert_eq!(a.signed_in_at, None);
}

#[test]
fn a_repo_that_refuses_the_login_is_not_a_rejected_login() {
    let (g, t) = forge();
    let runs = format!("{API}/repos/o/r/actions/runs?branch=b&per_page={RUN_LIMIT}");
    t.on("GET", &runs, Response::new(403, r#"{"message":"Resource not accessible by personal access token"}"#));
    let e = g.branch_runs("o/r", "b").unwrap_err();
    assert!(matches!(&e, ForgeError::Failed(m) if m.contains("403") && m.contains("not accessible")), "{e:?}");
    assert_eq!((g.login_needed(), g.login_rejected_at()), (false, None));
    let _ = g.branch_runs("o/r", "b");
    assert_eq!(t.sent().len(), 2, "still calling");
}

#[test]
fn a_refusal_of_the_old_token_does_not_mark_a_new_login() {
    let (g, _) = forge();
    g.set_account(Some(Account {
        token: "gho_new".into(),
        api_url: API.into(),
        login: "me".into(),
        signed_in_at: None,
    }));
    g.reject("gho_t");
    assert_eq!(g.login_rejected_at(), None);
    g.reject("gho_new");
    assert!(g.login_rejected_at().is_some());
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
    assert_eq!((r.id.as_str(), &r.reason, r.by_queue), ("RFMQE_x", &QueueRemovalReason::Conflict, true));
    removed["timelineItems"]["nodes"][0]["actor"]["login"] = json!("someone");
    assert!(!parse_pr(&removed).queue_removal.unwrap().by_queue);
    removed["timelineItems"]["nodes"][0]["__typename"] = json!("AddedToMergeQueueEvent");
    assert_eq!(parse_pr(&removed).queue_removal, None, "queued again since");
}

#[test]
fn merge_queue_reasons_actors_and_refs_are_githubs() {
    assert_eq!(queue_removal_reason("failed_checks"), QueueRemovalReason::FailedChecks);
    assert_eq!(queue_removal_reason("merged"), QueueRemovalReason::Merged);
    assert_eq!(queue_removal_reason("timeout"), QueueRemovalReason::Other("timeout".into()));
    assert!(is_queue_actor("github-merge-queue[bot]") && !is_queue_actor("someone"));
    let g = GithubApi::offline();
    assert_eq!(g.queue_ref("main", 12).as_deref(), Some("gh-readonly-queue/main/pr-12-"));
    assert_eq!(g.queue_name(), "merge queue");
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
    let ci = GithubActions::new(g.clone());
    let runs = ci.runs("o/r", "pdt/x", Some("abc")).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].provider.as_str(), runs[0].id.as_str()), ("github_actions", "391"));
    assert_eq!((runs[0].attempt, runs[0].state), (2, CiState::Failed));
    assert_eq!(
        (runs[0].name.as_str(), runs[0].head_sha.as_str(), runs[0].url.as_str(), runs[0].branch.as_str()),
        ("ci", "abc", "https://r/391", "pdt/x")
    );
    assert_eq!(runs[1].state, CiState::Running);

    t.ok(
        "GET",
        &format!("{API}/repos/o/r/actions/runs/391/attempts/2/jobs"),
        r#"{"jobs":[{"id":1,"name":"changes","status":"completed","conclusion":"success"},
          {"id":2,"name":"test (linux)","status":"completed","conclusion":"failure","html_url":"j2"},
          {"id":3,"name":"release","status":"completed","conclusion":"skipped"},
          {"id":4,"name":"approve","status":"completed","conclusion":"action_required"}]}"#,
    );
    let jobs = ci.jobs("o/r", &runs[0]).unwrap();
    assert_eq!(jobs.len(), 4);
    assert_eq!((jobs[1].id.as_str(), jobs[1].state, jobs[1].url.as_str()), ("2", CiState::Failed, "j2"));
    assert_eq!(jobs[3].state, CiState::Cancelled, "only real failures are failures");
    t.ok("GET", &format!("{API}/repos/o/r/actions/runs?event=merge_group"), r#"{"workflow_runs":[]}"#);
    assert!(ci.queue_runs("o/r").unwrap().is_empty());
    t.on("POST", &format!("{API}/repos/o/r/actions/runs/391/rerun-failed-jobs"), Response::new(201, "{}"));
    ci.retry("o/r", &runs[0], true).unwrap();
    t.on("POST", &format!("{API}/repos/o/r/actions/runs/391/rerun"), Response::new(201, "{}"));
    ci.retry("o/r", &runs[0], false).unwrap();
    assert_eq!(ci.config_paths(), pdt_forge::CiConfig::Paths(&[".github/workflows"]));
}

#[test]
fn failed_logs_are_the_tails_of_the_failed_jobs() {
    let (g, t) = forge();
    let ci = GithubActions::new(g.clone());
    let job =
        |id: &str| pdt_core::pr::CiJob { id: id.into(), name: "test".into(), state: CiState::Failed, url: "j".into() };
    let run = CiRun {
        provider: "github_actions".into(),
        id: "391".into(),
        attempt: 1,
        name: "ci".into(),
        state: CiState::Failed,
        url: String::new(),
        head_sha: String::new(),
        branch: String::new(),
        created_at: String::new(),
        failed_jobs: vec![],
    };
    let log: String = (1..=250).map(|i| format!("2026-10-19T02:00:10.1234567Z line {i}\n")).collect();
    t.ok(
        "GET",
        &format!("{API}/repos/o/r/actions/jobs/2/logs"),
        &format!("{log}2026-10-19T02:00:11.0Z \u{1b}[31merror\u{1b}[0m: boom\n"),
    );
    let lines = format!("{log}2026-10-19T02:00:11.0Z \u{1b}[31merror\u{1b}[0m: boom\n");
    t.ok("GET", &format!("{API}/repos/o/r/actions/jobs/2/logs"), &lines);
    let log = ci.job_log_tail("o/r", &run, &job("2"), 200).unwrap();
    assert!(log.starts_with("… (51 earlier lines)\nline 52\n"), "{}", &log[..60]);
    assert!(log.ends_with("line 250\nerror: boom\n"), "timestamps and colours dropped");
    assert!(ci.job_log_tail("o/r", &run, &job("2"), 10).unwrap().starts_with("… (241 earlier lines)"));
    assert!(ci.job_log_tail("o/r", &run, &job("3"), 200).unwrap().contains("no log"), "job 3's log is gone (404)");
    assert!(!t.sent()[0].headers.iter().any(|(k, _)| *k == "If-None-Match"), "logs are not cached");
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

    t.ok("POST", &format!("{API}/repos/o/r/issues"), &one.replace("\"closed\"", "\"open\""));
    let new = NewIssue { title: "Crash".into(), body: "It fell over".into(), labels: vec!["bug".into()] };
    let made = g.create_issue("o/r", &new).unwrap();
    assert_eq!((made.number, made.url.as_str()), (12, "https://github.com/o/r/issues/12"));
    assert_eq!(body_of(t.sent().last().unwrap()), json!({"title": "Crash", "body": "It fell over", "labels": ["bug"]}));
    t.ok("POST", &format!("{API}/repos/o/r/issues"), one);
    g.create_issue("o/r", &NewIssue { labels: vec![], ..new }).unwrap();
    assert_eq!(body_of(t.sent().last().unwrap()), json!({"title": "Crash", "body": "It fell over"}), "no labels");

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
fn reads_releases_and_starts_workflows() {
    let (g, t) = forge();
    t.ok(
        "GET",
        &format!("{API}/repos/o/r/releases/latest"),
        r#"{"tag_name":"v0.2.0","name":"","html_url":"https://github.com/o/r/releases/tag/v0.2.0",
          "published_at":"2026-10-28T10:00:00Z","body":"Added things","draft":false,"prerelease":false}"#,
    );
    let r = g.release("o/r", None).unwrap().unwrap();
    assert_eq!((r.tag.as_str(), r.name.as_str(), r.notes.as_str()), ("v0.2.0", "v0.2.0", "Added things"));
    assert_eq!(r.published_at.as_deref(), Some("2026-10-28T10:00:00Z"));
    t.on("GET", &format!("{API}/repos/o/r/releases/tags/v9.9.9"), Response::new(404, r#"{"message":"Not Found"}"#));
    assert_eq!(g.release("o/r", Some("v9.9.9")).unwrap(), None, "a 404");
    t.ok("GET", &format!("{API}/repos/o/r/releases/tags/v1.0.0"), r#"{"tag_name":"v1.0.0","prerelease":true}"#);
    assert_eq!(g.release("o/r", Some("v1.0.0")).unwrap(), None, "a pre-release is not a release");
    t.ok(
        "GET",
        &format!("{API}/repos/o/r/releases?per_page=10"),
        r#"[{"tag_name":"v0.3.0","draft":true},{"tag_name":"v0.2.1-exp.2","prerelease":true,"draft":false},
            {"tag_name":"v0.2.0"}]"#,
    );
    let r = g.newest_release("o/r").unwrap().unwrap();
    assert_eq!(r.tag, "v0.2.1-exp.2", "the newest, a pre-release too, never a draft");

    t.on("POST", &format!("{API}/repos/o/r/actions/workflows/release.yml/dispatches"), Response::new(204, ""));
    let inputs = BTreeMap::from([("bump".to_string(), "minor".to_string())]);
    let ci = GithubActions::new(g.clone());
    ci.trigger("o/r", "main", "release.yml", &inputs).unwrap();
    assert_eq!(body_of(t.sent().last().unwrap()), json!({"ref": "main", "inputs": {"bump": "minor"}}));
    t.on(
        "POST",
        &format!("{API}/repos/o/r/actions/workflows/nope.yml/dispatches"),
        Response::new(404, r#"{"message":"Not Found"}"#),
    );
    let e = ci.trigger("o/r", "main", "nope.yml", &inputs).unwrap_err();
    assert!(matches!(&e, ForgeError::Failed(m) if m.contains("no workflow nope.yml")), "{e:?}");
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
        signed_in_at: None,
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
