//! The GitLab forge and GitLab CI against recorded answers of GitLab's API (never the network). Answers are matched by
//! the start of their URL, in the order they are given: each test gives them in the order the calls are made.

use crate::api::{GitlabApi, GlError, parse_project};
use crate::ci::{GITLAB_CI, GitlabCi, log_tail, state_of};
use crate::forge::{car_on_train, parse_diff, parse_mr, pick_mr, train_ref, train_removal};
use crate::plugin::{GITLAB, GitlabPlugin, project_of};
use pdt_core::pr::{CiState, Mergeable, PrState, QueueRemovalReason};
use pdt_forge::login::Refresh;
use pdt_forge::transport::Response;
use pdt_forge::transport::testing::Recorded;
use pdt_forge::{
    CiProvider, CloseReason, Forge, ForgeAccount, ForgeError, ForgePlugin, ForgeRegistry, MergeStrategy,
    NewChangeRequest, NewIssue, ReviewVerdict,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const API: &str = "https://gitlab.com/api/v4";
const P: &str = "https://gitlab.com/api/v4/projects/team%2Fapp";

fn account(token: &str) -> ForgeAccount {
    ForgeAccount { token: token.into(), api_url: API.into(), login: "rwest".into(), ..Default::default() }
}

fn api(t: &Arc<Recorded>) -> Arc<GitlabApi> {
    let g = Arc::new(GitlabApi::new(t.clone()));
    g.set_account(Some(account("glpat-1")));
    g
}

fn project(trains: bool) -> String {
    json!({"id": 9, "path_with_namespace": "team/app", "merge_method": "merge", "squash_option": "default_on",
           "merge_pipelines_enabled": trains, "merge_trains_enabled": trains})
    .to_string()
}

fn mr(iid: u32, state: &str, sha: &str) -> Value {
    json!({
        "id": 1000 + iid, "iid": iid, "project_id": 9, "source_project_id": 9, "title": "PDT-1: thing",
        "description": "Body", "state": state, "draft": false, "sha": sha, "source_branch": "pdt-PDT-1-thing",
        "target_branch": "main", "detailed_merge_status": "mergeable", "has_conflicts": false,
        "merge_when_pipeline_succeeds": false, "created_at": "2026-10-29T10:00:00.000Z",
        "updated_at": "2026-10-29T11:00:00.000Z", "web_url": format!("https://gitlab.com/team/app/-/merge_requests/{iid}"),
        "author": {"id": 7, "username": "rwest"}, "labels": ["pdt"]
    })
}

fn pipeline(id: u64, sha: &str, git_ref: &str, status: &str, at: &str) -> Value {
    json!({"id": id, "iid": id - 100, "project_id": 9, "sha": sha, "ref": git_ref, "status": status,
           "source": "push", "created_at": at, "web_url": format!("https://gitlab.com/team/app/-/pipelines/{id}")})
}

#[test]
fn a_branchs_merge_request_is_its_newest_open_one_from_the_project_itself() {
    let t = Arc::new(Recorded::new());
    let mut fork = mr(9, "opened", "f0");
    fork["source_project_id"] = json!(44);
    let list = json!([fork, mr(8, "closed", "old"), mr(12, "opened", "abc")]);
    t.ok("GET", &format!("{P}/merge_requests?source_branch=pdt-PDT-1-thing"), &list.to_string()).ok(
        "GET",
        P,
        &project(false),
    );
    let g = api(&t);
    let cr = g.change_request("team/app", "pdt-PDT-1-thing").unwrap().unwrap();
    assert_eq!(
        (cr.number, cr.state, cr.head_sha.as_str(), cr.base_branch.as_str()),
        (12, PrState::Open, "abc", "main")
    );
    assert_eq!(cr.url, "https://gitlab.com/team/app/-/merge_requests/12");
    assert_eq!((cr.mergeable, cr.in_merge_queue, cr.created_at), (Mergeable::Mergeable, false, Some(1793268000000)));
    let sent = t.sent();
    assert!(sent[0].url.contains("state=all"), "{}", sent[0].url);
    assert!(sent[0].headers.contains(&("Authorization", "Bearer glpat-1".into())));
    assert_eq!(sent.len(), 2, "no merge-train reads without merge trains: {:?}", t.urls());

    let none = json!([]).to_string();
    t.ok("GET", &format!("{P}/merge_requests?"), &none);
    assert_eq!(g.change_request("team/app", "nothing").unwrap(), None);
}

#[test]
fn merge_requests_say_whether_they_can_merge() {
    let with = |k: &str, v: Value| {
        let mut m = mr(1, "opened", "a");
        m[k] = v;
        parse_mr(&m)
    };
    assert_eq!(with("has_conflicts", json!(true)).mergeable, Mergeable::Conflicting);
    assert_eq!(with("detailed_merge_status", json!("checking")).mergeable, Mergeable::Unknown);
    assert_eq!(with("detailed_merge_status", json!("ci_must_pass")).mergeable, Mergeable::Mergeable);
    let behind = with("detailed_merge_status", json!("need_rebase"));
    assert!(behind.behind && behind.mergeable == Mergeable::Mergeable);
    assert!(with("merge_when_pipeline_succeeds", json!(true)).auto_merge);
    assert!(with("draft", json!(true)).draft);
    assert_eq!(parse_mr(&mr(1, "merged", "a")).state, PrState::Merged);
    assert_eq!(parse_mr(&mr(1, "locked", "a")).state, PrState::Closed);
    assert_eq!(
        pick_mr(&[mr(3, "merged", "x"), mr(2, "closed", "y")]).unwrap()["iid"],
        3,
        "the newest when none is open"
    );
}

#[test]
fn merge_trains_are_found_per_project_and_queue_a_merge_request() {
    let t = Arc::new(Recorded::new());
    let list = json!([mr(12, "opened", "abc")]).to_string();
    t.ok("GET", &format!("{P}/merge_requests?source_branch="), &list)
        .ok("GET", P, &project(true))
        .ok(
            "GET",
            &format!("{P}/merge_trains/merge_requests/12"),
            r#"{"id":3,"status":"fresh","target_branch":"main"}"#,
        )
        .ok(
            "GET",
            &format!("{P}/merge_trains/main?scope=active"),
            r#"[{"id":2,"merge_request":{"iid":10}},{"id":3,"merge_request":{"iid":12}}]"#,
        );
    let g = api(&t);
    let cr = g.change_request("team/app", "pdt-PDT-1-thing").unwrap().unwrap();
    assert_eq!((cr.in_merge_queue, cr.queue_position), (true, Some(2)));
    assert_eq!(g.queue_name(), "merge train");
    assert_eq!(g.queue_ref("main", 12).as_deref(), Some("refs/merge-requests/12/train"));
    let s = g.merge_settings("team/app", "main").unwrap();
    assert!(s.queue && s.squash && s.merge && !s.rebase);
    assert_eq!(s.strategy(), None, "the train picks the method");

    // Taken off the train because its pipeline failed: the newest train note says so.
    t.ok("GET", &format!("{P}/merge_requests?source_branch="), &list)
        .on("GET", &format!("{P}/merge_trains/merge_requests/12"), Response::new(404, r#"{"message":"404 Not found"}"#))
        .ok(
            "GET",
            &format!("{P}/merge_requests/12/notes"),
            r#"[{"id":81,"system":true,"body":"removed this merge request from the merge train because the pipeline did not succeed."},
                {"id":80,"system":true,"body":"added this merge request to the merge train at position 2"}]"#,
        );
    let cr = g.change_request("team/app", "pdt-PDT-1-thing").unwrap().unwrap();
    let r = cr.queue_removal.unwrap();
    assert_eq!(
        (cr.in_merge_queue, r.reason, r.by_queue, r.id.as_str()),
        (false, QueueRemovalReason::FailedChecks, true, "note:81")
    );

    // Merging puts it on the train, only while its head is the approved one.
    t.ok("POST", &format!("{P}/merge_trains/merge_requests/12"), "[]");
    g.merge_head("team/app", 12, None, Some("abc")).unwrap();
    let sent = t.sent();
    let body: Value = serde_json::from_str(sent.last().unwrap().body.as_deref().unwrap()).unwrap();
    assert_eq!(body, json!({"auto_merge": true, "when_pipeline_succeeds": true, "sha": "abc"}));
}

#[test]
fn train_notes_and_cars() {
    let note = |id: u64, body: &str| json!({"id": id, "system": true, "body": body});
    assert_eq!(train_removal(&[note(1, "added this merge request to the merge train at position 1")]), None);
    let hand = train_removal(&[note(2, "removed this merge request from the merge train")]).unwrap();
    assert!(!hand.by_queue, "by hand");
    let conflict =
        train_removal(&[note(3, "removed this merge request from the merge train because it has conflicts")]);
    assert_eq!(conflict.unwrap().reason, QueueRemovalReason::Conflict);
    assert!(
        train_removal(&[json!({"id": 4, "system": false, "body": "removed from the merge train because x"})]).is_none()
    );
    assert!(car_on_train(&json!({"status": "idle"})) && !car_on_train(&json!({"status": "merged"})));
    assert_eq!(train_ref(7), "refs/merge-requests/7/train");
    let p = parse_project(&json!({"merge_trains_enabled": true, "merge_pipelines_enabled": false}));
    assert!(!p.merge_trains && !p.merged_results, "trains need merged-results pipelines");
}

#[test]
fn a_merge_without_a_train_merges_when_the_pipeline_succeeds() {
    let t = Arc::new(Recorded::new());
    t.ok("GET", P, &project(false)).ok(
        "PUT",
        &format!("{P}/merge_requests/12/merge"),
        &mr(12, "opened", "abc").to_string(),
    );
    let g = api(&t);
    g.merge("team/app", 12, Some(MergeStrategy::Squash)).unwrap();
    let body: Value = serde_json::from_str(t.sent()[1].body.as_deref().unwrap()).unwrap();
    assert_eq!(body, json!({"auto_merge": true, "merge_when_pipeline_succeeds": true, "squash": true}));

    t.on(
        "PUT",
        &format!("{P}/merge_requests/12/merge"),
        Response::new(409, r#"{"message":"SHA does not match HEAD of source branch: def"}"#),
    )
    .on(
        "PUT",
        &format!("{P}/merge_requests/12/merge"),
        Response::new(405, r#"{"message":"405 Method Not Allowed"}"#),
    );
    let e = g.merge_head("team/app", 12, None, Some("abc")).unwrap_err();
    assert!(matches!(&e, ForgeError::Failed(m) if m.contains("head of !12 moved")), "{e:?}");
    let e = g.merge("team/app", 12, None).unwrap_err();
    assert!(e.to_string().contains("can't merge !12 now"), "{e}");
}

#[test]
fn a_merge_request_opens_as_a_draft_or_not() {
    let t = Arc::new(Recorded::new());
    t.on("POST", &format!("{P}/merge_requests"), Response::new(201, mr(13, "opened", "").to_string())).on(
        "POST",
        &format!("{P}/merge_requests"),
        Response::new(409, r#"{"message":["Another open merge request already exists for this source branch: !13"]}"#),
    );
    let g = api(&t);
    let new = NewChangeRequest {
        head: "pdt-PDT-1-thing".into(),
        base: "main".into(),
        title: "PDT-1: thing".into(),
        body: "What changed".into(),
        draft: true,
    };
    let o = g.open_change_request("team/app", &new).unwrap();
    assert_eq!((o.number, o.url.as_str()), (13, "https://gitlab.com/team/app/-/merge_requests/13"));
    let body: Value = serde_json::from_str(t.sent()[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(body["title"], "Draft: PDT-1: thing");
    assert_eq!(
        (body["source_branch"].as_str(), body["description"].as_str()),
        (Some("pdt-PDT-1-thing"), Some("What changed"))
    );
    let e = g.open_change_request("team/app", &new).unwrap_err();
    assert!(e.to_string().contains("Another open merge request already exists"), "{e}");
}

fn discussions() -> String {
    json!([
        {"id": "d1", "individual_note": true, "notes": [
            {"id": 501, "type": null, "body": "Looks good", "system": false, "author": {"id": 7, "username": "rwest"}}]},
        {"id": "d2", "individual_note": false, "notes": [
            {"id": 502, "type": "DiffNote", "body": "Rename this", "system": false, "author": {"id": 8, "username": "al"},
             "position": {"new_path": "src/lib.rs", "new_line": 12, "old_path": "src/lib.rs", "old_line": null}},
            {"id": 503, "type": "DiscussionNote", "body": "ok", "system": false, "author": {"id": 7, "username": "rwest"}}]},
        {"id": "d3", "individual_note": true, "notes": [
            {"id": 504, "body": "added 1 commit", "system": true, "author": {"id": 7, "username": "rwest"}}]},
        {"id": "d4", "individual_note": true, "notes": [
            {"id": 505, "body": "Pipeline passed", "system": false, "author": {"id": 99, "username": "project_9_bot_abc"}}]}
    ])
    .to_string()
}

#[test]
fn discussions_are_comments_and_replies_go_into_them() {
    let t = Arc::new(Recorded::new());
    t.ok("GET", &format!("{P}/merge_requests/12/discussions"), &discussions())
        .ok("GET", &format!("{P}/members/all/7"), r#"{"id":7,"access_level":40}"#)
        .ok("GET", &format!("{P}/members/all/8"), r#"{"id":8,"access_level":20}"#)
        .on("GET", &format!("{P}/members/all/99"), Response::new(404, r#"{"message":"404 Not found"}"#));
    let g = api(&t);
    let c = g.change_comments("team/app", 12).unwrap();
    let ids: Vec<u64> = c.iter().map(|c| c.id).collect();
    assert_eq!(ids, [501, 502, 503, 505], "system notes left out");
    assert_eq!(c[0].url, "https://gitlab.com/team/app/-/merge_requests/12#note_501");
    assert!(c[0].collaborator && !c[0].in_review, "a Maintainer");
    let diff = &c[1];
    assert_eq!((diff.path.as_deref(), diff.line, diff.in_review), (Some("src/lib.rs"), Some(12), true));
    assert!(!diff.collaborator, "a Reporter is not an insider");
    assert!(c[3].is_bot && !c[3].collaborator);
    assert_eq!(t.sent().iter().filter(|r| r.url.contains("/members/all/7")).count(), 1, "asked once per user");

    t.ok("GET", &format!("{P}/merge_requests/12/discussions"), &discussions()).on(
        "POST",
        &format!("{P}/merge_requests/12/discussions/d2/notes"),
        Response::new(201, r#"{"id":600,"body":"Done"}"#),
    );
    let url = g.reply_review_comment("team/app", 12, 502, "Done").unwrap();
    assert_eq!(url, "https://gitlab.com/team/app/-/merge_requests/12#note_600");
    assert!(g.reply_review_comment("team/app", 12, 999, "x").is_err(), "no such comment");

    t.on("POST", &format!("{P}/merge_requests/12/notes"), Response::new(201, r#"{"id":601}"#));
    let url = g.comment_change_request("team/app", 12, "Hello").unwrap();
    assert!(url.ends_with("#note_601"), "{url}");
}

#[test]
fn approvals_and_reviewers_are_reviews() {
    let t = Arc::new(Recorded::new());
    t.ok("GET", &format!("{P}/merge_requests/12/approvals"), r#"{"approved_by":[{"user":{"id":8,"username":"al"}}]}"#)
        .ok("GET", &format!("{P}/members/all/8"), r#"{"access_level":30}"#)
        .ok(
            "GET",
            &format!("{P}/merge_requests/12/reviewers"),
            r#"[{"user":{"id":8,"username":"al"},"state":"approved"},
                {"user":{"id":9,"username":"bo"},"state":"requested_changes"},
                {"user":{"id":10,"username":"cy"},"state":"unreviewed"}]"#,
        );
    let g = api(&t);
    let r = g.change_reviews("team/app", 12).unwrap();
    let got: Vec<(&str, &str, bool)> =
        r.iter().map(|r| (r.author.as_str(), r.state.as_str(), r.collaborator)).collect();
    assert_eq!(got, [("al", "approved", true), ("bo", "changes_requested", false)]);

    t.on("POST", &format!("{P}/merge_requests/12/approve"), Response::new(201, "{}")).on(
        "POST",
        &format!("{P}/merge_requests/12/notes"),
        Response::new(201, r#"{"id":700}"#),
    );
    assert_eq!(
        g.review_change_request("team/app", 12, ReviewVerdict::Approve, "").unwrap(),
        "https://gitlab.com/team/app/-/merge_requests/12"
    );
    let url = g.review_change_request("team/app", 12, ReviewVerdict::RequestChanges, "Fix the tests").unwrap();
    assert!(url.ends_with("#note_700"));
    let note: Value = serde_json::from_str(t.sent().last().unwrap().body.as_deref().unwrap()).unwrap();
    assert!(note["body"].as_str().unwrap().contains("Changes requested"), "{note}");
}

#[test]
fn diffs_are_files_and_stats() {
    let t = Arc::new(Recorded::new());
    t.ok(
        "GET",
        &format!("{P}/merge_requests/12/diffs"),
        r#"[{"old_path":"a.rs","new_path":"a.rs","diff":"@@ -1,2 +1,2 @@\n-old\n+new\n+more\n ctx\n","new_file":false,"renamed_file":false,"deleted_file":false},
            {"old_path":"b.rs","new_path":"c.rs","diff":"","new_file":false,"renamed_file":true,"deleted_file":false},
            {"old_path":"big.bin","new_path":"big.bin","diff":"","new_file":true,"too_large":true}]"#,
    )
    .ok("GET", &format!("{P}/merge_requests/12/diffs"), r#"[{"new_path":"x","diff":"+a\n"}]"#);
    let g = api(&t);
    let f = g.change_request_files("team/app", 12).unwrap();
    assert_eq!((f[0].status.as_str(), f[0].additions, f[0].deletions), ("modified", 2, 1));
    assert_eq!(
        (f[1].status.as_str(), f[1].previous_path.as_deref(), f[1].patch.as_deref()),
        ("renamed", Some("b.rs"), None)
    );
    assert_eq!((f[2].status.as_str(), f[2].patch.as_deref()), ("added", None));
    let s = g.change_request_stats("team/app", 12).unwrap();
    assert_eq!((s.changed_files, s.additions, s.deletions), (1, 1, 0));
    assert!(parse_diff(&json!({"new_path": ""})).is_none());
}

#[test]
fn issues_are_read_opened_commented_labelled_and_closed() {
    let t = Arc::new(Recorded::new());
    let issue = |iid: u64| {
        json!({"id": 5000 + iid, "iid": iid, "title": format!("Issue {iid}"), "description": "Body", "state": "opened",
               "web_url": format!("https://gitlab.com/team/app/-/issues/{iid}"), "author": {"username": "al"},
               "labels": ["bug"], "created_at": "2026-10-29T10:00:00.000Z"})
    };
    t.ok("GET", &format!("{P}/issues?scope=all"), &json!([issue(3), issue(2)]).to_string())
        .ok("GET", &format!("{P}/issues/3"), &issue(3).to_string())
        .on("GET", &format!("{P}/issues/4"), Response::new(404, r#"{"message":"404 Not found"}"#))
        .on("POST", &format!("{P}/issues"), Response::new(201, issue(6).to_string()))
        .on("PUT", &format!("{P}/issues/3"), Response::new(200, issue(3).to_string()))
        .on("POST", &format!("{P}/issues/2/notes"), Response::new(201, r#"{"id":90}"#))
        .on("PUT", &format!("{P}/issues/3"), Response::new(200, issue(3).to_string()));
    let g = api(&t);
    let list = g.recent_issues("team/app", None).unwrap();
    assert_eq!(list.iter().map(|i| i.number).collect::<Vec<_>>(), [3, 2]);
    assert_eq!((list[0].body.as_str(), list[0].labels.clone()), ("Body", vec!["bug".to_string()]));
    let d = g.issue("team/app", 3).unwrap().unwrap();
    assert_eq!((d.id, d.closed), (3, false), "its id is its iid, what /duplicate names");
    assert_eq!(g.issue("team/app", 4).unwrap(), None);
    let new = NewIssue { title: "Crash".into(), body: "It fell over".into(), labels: vec!["bug".into(), "p1".into()] };
    assert_eq!(g.create_issue("team/app", &new).unwrap().number, 6);
    let body: Value = serde_json::from_str(t.sent()[3].body.as_deref().unwrap()).unwrap();
    assert_eq!((body["labels"].as_str(), body["description"].as_str()), (Some("bug,p1"), Some("It fell over")));
    g.add_labels("team/app", 3, &["triaged".into()]).unwrap();
    assert_eq!(t.sent()[4].body.as_deref(), Some(r#"{"add_labels":"triaged"}"#));
    g.close_issue("team/app", 2, CloseReason::Duplicate, Some(3)).unwrap();
    assert_eq!(t.sent()[5].body.as_deref(), Some(r##"{"body":"/duplicate #3"}"##));
    g.close_issue("team/app", 3, CloseReason::NotPlanned, None).unwrap();
    assert_eq!(t.sent()[6].body.as_deref(), Some(r#"{"state_event":"close"}"#));
}

#[test]
fn releases_are_the_latest_or_a_tags_and_never_an_upcoming_one() {
    let t = Arc::new(Recorded::new());
    let rel = |tag: &str, upcoming: bool| {
        json!({"tag_name": tag, "name": "", "description": "Notes", "released_at": "2026-10-28T00:00:00Z",
               "upcoming_release": upcoming, "_links": {"self": format!("https://gitlab.com/team/app/-/releases/{tag}")}})
    };
    t.ok("GET", &format!("{P}/releases/permalink/latest"), &rel("v1.2.0", false).to_string())
        .ok("GET", &format!("{P}/releases/v2.0.0"), &rel("v2.0.0", true).to_string())
        .ok("GET", &format!("{P}/releases?"), &json!([rel("v2.0.0", true), rel("v1.2.0", false)]).to_string());
    let g = api(&t);
    let r = g.release("team/app", None).unwrap().unwrap();
    assert_eq!((r.tag.as_str(), r.name.as_str(), r.notes.as_str()), ("v1.2.0", "v1.2.0", "Notes"));
    assert_eq!(r.url, "https://gitlab.com/team/app/-/releases/v1.2.0");
    assert_eq!(g.release("team/app", Some("v2.0.0")).unwrap(), None, "not out yet");
    assert_eq!(g.newest_release("team/app").unwrap().unwrap().tag, "v1.2.0");
    assert_eq!(g.release("team/app", Some("v9")).unwrap(), None, "a 404");
}

#[test]
fn pipelines_are_the_ci_of_a_commit_with_merged_results_counted_as_the_head() {
    let t = Arc::new(Recorded::new());
    let mut merged = pipeline(203, "merge-sha", "refs/merge-requests/12/merge", "failed", "2026-10-29T10:05:00.000Z");
    merged["source"] = json!("merge_request_event");
    let mut detail = mr(12, "opened", "abc");
    detail["head_pipeline"] = merged;
    t.ok(
        "GET",
        &format!("{P}/pipelines?sha=abc"),
        &json!([pipeline(202, "abc", "pdt-PDT-1-thing", "success", "2026-10-29T10:00:00.000Z")]).to_string(),
    )
    .ok("GET", P, &project(true))
    .ok(
        "GET",
        &format!("{P}/merge_requests?source_branch=pdt-PDT-1-thing&state=opened"),
        &json!([mr(12, "opened", "abc")]).to_string(),
    )
    .ok("GET", &format!("{P}/merge_requests/12"), &detail.to_string());
    let ci = GitlabCi::new(api(&t));
    assert_eq!((ci.id(), ci.label()), (GITLAB_CI, "GitLab CI"));
    assert!(ci.capabilities().branches, "every pushed branch gets a pipeline");
    let runs = ci.runs("team/app", "pdt-PDT-1-thing", Some("abc")).unwrap();
    let got: Vec<(&str, &str, CiState)> = runs.iter().map(|r| (r.id.as_str(), r.head_sha.as_str(), r.state)).collect();
    assert_eq!(got, [("202", "abc", CiState::Passed), ("203", "abc", CiState::Failed)]);
    let status = pdt_core::pr::CiStatus::of(&runs, "abc").unwrap();
    assert_eq!(status.state, CiState::Failed, "the newest pipeline of the head is the merged result's");

    // Without merged-results pipelines, only the commit's own are read.
    let t = Arc::new(Recorded::new());
    t.ok("GET", &format!("{P}/pipelines?ref=main"), "[]").ok("GET", &format!("{P}/pipelines?sha=abc"), "[]").ok(
        "GET",
        P,
        &project(false),
    );
    let ci = GitlabCi::new(api(&t));
    assert!(ci.runs("team/app", "main", None).unwrap().is_empty());
    assert!(ci.runs("team/app", "b", Some("abc")).unwrap().is_empty());
    assert_eq!(t.sent().len(), 3, "{:?}", t.urls());
}

#[test]
fn jobs_logs_retries_and_new_pipelines() {
    let t = Arc::new(Recorded::new());
    t.ok(
        "GET",
        &format!("{P}/pipelines/202/jobs"),
        r#"[{"id":1,"name":"test","status":"failed","allow_failure":false,"web_url":"https://gitlab.com/team/app/-/jobs/1"},
            {"id":2,"name":"lint","status":"failed","allow_failure":true,"web_url":"u2"},
            {"id":3,"name":"build","status":"success","web_url":"u3"}]"#,
    )
    .ok(
        "GET",
        &format!("{P}/jobs/1/trace"),
        "\u{1b}[0Ksection_start:1:step_script\r\u{1b}[0K\u{1b}[32;1m$ cargo test\u{1b}[0;m\nrunning 3 tests\r\ntest a ... ok\ndownloading 10%\rdownloading 100%\nerror: test failed\n",
    )
    .on("GET", &format!("{P}/jobs/9/trace"), Response::new(404, r#"{"message":"404 Not found"}"#))
    .on("POST", &format!("{P}/pipelines/202/retry"), Response::new(201, "{}"))
    .on("POST", &format!("{P}/pipeline"), Response::new(201, "{}"))
    .on("POST", &format!("{P}/pipeline"), Response::new(201, "{}"));
    let ci = GitlabCi::new(api(&t));
    let run = crate::ci::parse_pipeline(&pipeline(202, "abc", "pdt-PDT-1-thing", "failed", "t")).unwrap();
    let jobs = ci.jobs("team/app", &run).unwrap();
    let states: Vec<(&str, CiState)> = jobs.iter().map(|j| (j.name.as_str(), j.state)).collect();
    assert_eq!(states, [("test", CiState::Failed), ("lint", CiState::Passed), ("build", CiState::Passed)]);
    let log = ci.job_log_tail("team/app", &run, &jobs[0], 4).unwrap();
    assert_eq!(log, "… (1 earlier lines)\nrunning 3 tests\ntest a ... ok\ndownloading 100%\nerror: test failed\n");
    let gone = pdt_core::pr::CiJob { id: "9".into(), ..jobs[0].clone() };
    assert!(ci.job_log_tail("team/app", &run, &gone, 4).unwrap().contains("no log"));
    ci.retry("team/app", &run, true).unwrap();
    ci.retry("team/app", &run, false).unwrap();
    assert_eq!(
        t.sent().last().unwrap().body.as_deref(),
        Some(r#"{"ref":"pdt-PDT-1-thing"}"#),
        "all jobs: a new pipeline"
    );
    let inputs = BTreeMap::from([("VERSION".to_string(), "1.2.0".to_string())]);
    ci.trigger("team/app", "main", "release", &inputs).unwrap();
    let body: Value = serde_json::from_str(t.sent().last().unwrap().body.as_deref().unwrap()).unwrap();
    assert_eq!(body, json!({"ref": "main", "variables": [{"key": "VERSION", "value": "1.2.0"}]}));
    assert_eq!(state_of("manual"), CiState::Queued, "waits for someone, as GitLab's merge check does");
    assert_eq!(state_of("canceled"), CiState::Cancelled);
    assert_eq!(log_tail("a\r\n", 10), "a\n");
}

#[test]
fn a_trains_pipelines_are_the_merge_queues_runs() {
    let t = Arc::new(Recorded::new());
    let mut car = pipeline(300, "train-sha", "refs/merge-requests/12/train", "running", "2026-10-29T12:00:00.000Z");
    car["source"] = json!("merge_request_event");
    t.ok("GET", &format!("{P}/pipelines?source=merge_request_event"), &json!([car]).to_string());
    let ci = GitlabCi::new(api(&t));
    let q = ci.queue_runs("team/app").unwrap();
    assert!(q[0].branch.starts_with(&train_ref(12)));
}

#[test]
fn a_refused_token_marks_the_login_until_the_user_signs_in_again() {
    let t = Arc::new(Recorded::new());
    let refused = || Response::new(401, r#"{"message":"401 Unauthorized"}"#);
    t.on("GET", &format!("{P}/issues/1"), refused()).on("GET", &format!("{API}/user"), refused());
    let g = api(&t);
    assert!(!g.login_needed());
    assert!(matches!(g.issue("team/app", 1), Err(ForgeError::Unavailable(_))));
    assert!(g.login_needed() && g.login_rejected_at().is_some());
    let n = t.sent().len();
    assert!(g.issue("team/app", 1).is_err());
    assert_eq!(t.sent().len(), n, "nothing is sent while it is refused");
    g.set_account(Some(account("glpat-2")));
    assert!(!g.login_needed());

    // One refused call that GitLab takes back when asked again leaves the login alone.
    t.on("GET", &format!("{P}/issues/1"), refused()).ok("GET", &format!("{API}/user"), r#"{"username":"rwest"}"#);
    assert!(matches!(g.issue("team/app", 1), Err(ForgeError::Failed(_))));
    assert!(g.login_rejected_at().is_none());

    let none = GitlabApi::new(t.clone());
    assert!(matches!(none.issue("team/app", 1), Err(ForgeError::Unavailable(m)) if m.contains("not connected")));
    assert!(none.login_needed(), "something wanted it");
}

#[test]
fn an_oauth_token_renews_before_it_expires_and_the_new_one_is_stored() {
    let t = Arc::new(Recorded::new());
    t.ok(
        "POST",
        "https://gitlab.com/oauth/token",
        r#"{"access_token":"glo_2","expires_in":7200,"refresh_token":"glr_2"}"#,
    )
    .ok("GET", &format!("{API}/user"), r#"{"username":"rwest"}"#);
    let g = GitlabApi::new(t.clone());
    let stored: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let s = stored.clone();
    g.on_renewed(Some(Arc::new(move |old: &str, a: &ForgeAccount| {
        s.lock().unwrap().push((old.to_string(), a.token.clone()));
    })));
    let soon = crate::api::now_ms() + 60_000;
    g.set_account(Some(ForgeAccount {
        expires_at: Some(soon),
        refresh: Some(Refresh { token: "glr_1".into(), client_id: "app".into(), expires_at: None }),
        ..account("glo_1")
    }));
    assert_eq!(g.viewer_login().unwrap(), "rwest");
    assert_eq!(*stored.lock().unwrap(), [("glo_1".to_string(), "glo_2".to_string())]);
    let a = g.account().unwrap();
    assert_eq!((a.token.as_str(), a.refresh.unwrap().token.as_str()), ("glo_2", "glr_2"));
    assert!(t.sent()[1].headers.contains(&("Authorization", "Bearer glo_2".into())));
}

#[test]
fn a_used_up_rate_limit_stops_calls_until_it_resets() {
    let t = Arc::new(Recorded::new());
    t.on("GET", &format!("{API}/user"), Response::new(429, "Retry later").with_header("Retry-After", "60"));
    let g = api(&t);
    assert!(matches!(g.viewer_login(), Err(ForgeError::Failed(m)) if m.contains("rate limit")));
    assert!(g.viewer_login().is_err());
    assert_eq!(t.sent().len(), 1, "the second call waits for the reset");
    assert!(matches!(ForgeError::from(GlError::RateLimited("x".into())), ForgeError::Failed(_)));
}

fn signed(plugin: &GitlabPlugin, host: &str, api_url: &str) {
    use pdt_forge::ForgeLogin;
    plugin.set_account(
        host,
        Some(ForgeAccount { token: "t".into(), api_url: api_url.into(), login: "me".into(), ..Default::default() }),
    );
}

#[test]
fn gitlab_claims_gitlab_com_and_the_servers_it_has_a_login_for() {
    let plugin = Arc::new(GitlabPlugin::new(Arc::new(GitlabApi::offline())));
    let r = ForgeRegistry::new(vec![plugin.clone() as Arc<dyn ForgePlugin>]);
    let repo = |url: &str| r.detect(url).map(|(_, r)| r);
    assert_eq!(repo("git@gitlab.com:team/app.git").as_deref(), Some("team/app"));
    assert_eq!(repo("https://gitlab.com/group/sub/app.git").as_deref(), Some("group/sub/app"), "nested groups");
    assert_eq!(repo("https://gitlab.com/team/app/-/tree/main"), None, "a page, not a project");
    assert_eq!(repo("https://gitlab.example.com/team/app.git"), None, "no login for it");
    signed(&plugin, "gitlab.example.com", "https://gitlab.example.com/api/v4");
    assert_eq!(repo("ssh://git@gitlab.example.com:2222/team/app.git").as_deref(), Some("team/app"));
    assert_eq!(r.get(&GITLAB).unwrap().label(), "GitLab");
    assert!(r.detect("git@github.com:o/r.git").is_none());
    let l = plugin.login().unwrap();
    assert_eq!(l.host_of(&l.api_url("")), "gitlab.com");
    assert_eq!(l.host_of(&l.api_url("https://GitLab.Example.com")), "gitlab.example.com");
    let hosts: Vec<String> = l.accounts().into_iter().map(|a| a.host).collect();
    assert_eq!(hosts, ["gitlab.example.com"], "gitlab.com is not signed in");
    let own = plugin.api_on("gitlab.example.com").unwrap();
    assert!(!Arc::ptr_eq(&own, plugin.api()), "a client of its own");
    l.set_account("gitlab.example.com", None);
    assert_eq!(repo("https://gitlab.example.com/team/app.git"), None, "signed out");
    assert!(project_of(&pdt_core::git_url::parse_remote("https://gitlab.com/a/b/c/d").unwrap()).is_some());
}

#[test]
fn links_are_on_the_servers_web_url() {
    let plugin = GitlabPlugin::new(Arc::new(GitlabApi::offline()));
    let remote = pdt_core::git_url::parse_remote("git@gitlab.com:group/sub/app.git").unwrap();
    let l = plugin.links(&remote, "group/sub/app").unwrap();
    assert_eq!(l.repo_url, "https://gitlab.com/group/sub/app");
    assert_eq!(l.branch_url.as_deref(), Some("https://gitlab.com/group/sub/app/-/tree/{branch}"));
    assert_eq!(
        l.compare_url.as_deref(),
        Some(
            "https://gitlab.com/group/sub/app/-/merge_requests/new?merge_request%5Bsource_branch%5D={branch}\
             &merge_request%5Btarget_branch%5D={base}"
        )
    );
    signed(&plugin, "git.example.org", "https://git.example.org:8443/api/v4");
    let remote = pdt_core::git_url::parse_remote("git@git.example.org:team/app.git").unwrap();
    assert_eq!(plugin.links(&remote, "team/app").unwrap().repo_url, "https://git.example.org:8443/team/app");
}

#[test]
fn merge_request_pages_are_change_requests() {
    let plugin = GitlabPlugin::new(Arc::new(GitlabApi::offline()));
    assert_eq!(plugin.change_request_name(), "merge request");
    assert!(plugin.is_change_request_url("https://gitlab.com/group/sub/app/-/merge_requests/12"));
    assert!(!plugin.is_change_request_url("https://gitlab.com/group/app/-/issues/12"));
    assert!(!plugin.is_change_request_url("https://gitlab.com/group/app/-/merge_requests/new"));
    assert!(!plugin.is_change_request_url("https://elsewhere.example/group/app/-/merge_requests/12"));
}

/// The ticket's whole run on a gitlab.com project: its merge request opened, its pipeline read on the head, then
/// merged when the pipeline succeeds, and seen merged.
#[test]
fn a_ticket_runs_end_to_end_merge_request_pipeline_merge() {
    let t = Arc::new(Recorded::new());
    let plugin = GitlabPlugin::new(api(&t));
    let remote = pdt_core::git_url::parse_remote("git@gitlab.com:team/app.git").unwrap();
    let repo = plugin.repo_of(&remote).unwrap();
    let (forge, ci) = (plugin.forge_on("gitlab.com"), plugin.ci_on("gitlab.com").unwrap());
    let branch = "pdt-PDT-1-thing";
    t.on("POST", &format!("{P}/merge_requests"), Response::new(201, mr(12, "opened", "abc").to_string()))
        .ok("GET", &format!("{P}/merge_requests?source_branch="), &json!([mr(12, "opened", "abc")]).to_string())
        .ok("GET", P, &project(false))
        .ok(
            "GET",
            &format!("{P}/pipelines?sha=abc"),
            &json!([pipeline(202, "abc", branch, "success", "t")]).to_string(),
        )
        .ok("PUT", &format!("{P}/merge_requests/12/merge"), &mr(12, "merged", "abc").to_string())
        .ok("GET", &format!("{P}/merge_requests?source_branch="), &json!([mr(12, "merged", "abc")]).to_string());
    let new = NewChangeRequest {
        head: branch.into(),
        base: "main".into(),
        title: "PDT-1".into(),
        body: String::new(),
        draft: false,
    };
    assert_eq!(forge.open_change_request(&repo, &new).unwrap().number, 12);
    let cr = forge.change_request(&repo, branch).unwrap().unwrap();
    let runs = ci.runs(&repo, branch, Some(&cr.head_sha)).unwrap();
    assert_eq!(pdt_core::pr::CiStatus::of(&runs, &cr.head_sha).unwrap().state, CiState::Passed);
    let strategy = forge.merge_settings(&repo, "main").unwrap().strategy();
    assert_eq!(strategy, Some(MergeStrategy::Squash));
    forge.merge_head(&repo, cr.number, strategy, Some(&cr.head_sha)).unwrap();
    assert_eq!(forge.change_request(&repo, branch).unwrap().unwrap().state, PrState::Merged);
    let calls: Vec<String> = t.urls().iter().map(|u| u.split('?').next().unwrap().replace(P, "")).collect();
    assert_eq!(
        calls,
        [
            "POST /merge_requests",
            "GET /merge_requests",
            "GET ",
            "GET /pipelines",
            "PUT /merge_requests/12/merge",
            "GET /merge_requests"
        ]
    );
}
