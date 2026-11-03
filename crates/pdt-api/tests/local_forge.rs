//! The local forge (PDT-178): a project with no remote, `forge = "local"` and a `[ci]` command. A started ticket's
//! commits open a change request, CI runs in its worktree, a failure is news for the agent, and once it is set to merge
//! and CI passes PDT merges it into the base itself. Nothing touches the network: there is no remote, and the GitHub
//! forge is the one that runs nothing.

use crate::common;

use axum::http::StatusCode;
use common::wait::wait_until;
use common::{argv_agent, call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

struct Env {
    app: axum::Router,
    state: AppState,
    repo: PathBuf,
    worktree: PathBuf,
    ticket: String,
    attempt: String,
}

/// A repository with no remote on the local forge, whose CI passes once `ok.txt` is committed, and one started ticket.
async fn setup(tmp: &Path, config: &str) -> Env {
    setup_with(tmp, config, true).await
}

/// [`setup`], with the configuration's commands approved or not (PDT-405).
async fn setup_with(tmp: &Path, config: &str, approved: bool) -> Env {
    let repo = tmp.join("repo");
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join(".pdt/project.toml"), config).unwrap();
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp);
    state.platform.claude_config_dir = Some(tmp.join("claude-config"));
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    tokio::spawn(pdt_api::run_local_ci_watch(state.clone()));
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "Offline", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let pid = p["id"].as_str().unwrap();
    if approved {
        common::approve_commands(&app, pid, None).await;
    }
    let (_, t) = call(&app, "POST", &format!("/api/projects/{pid}/tickets"), Some(json!({"title": "Ship it"}))).await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let (s, started) = call(
        &app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "sessionMode": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let attempt = started["attempt"]["id"].as_str().unwrap().to_string();
    let wt = started["attempt"]["worktreeId"].as_str().unwrap();
    let worktree = PathBuf::from(state.store.get_worktree(wt).await.unwrap().unwrap().path);
    Env { app, state, repo, worktree, ticket, attempt }
}

const CI: &str = "forge = \"local\"\n\n[ci]\ncommand = \"git ls-files --error-unmatch ok.txt\"\n";

fn commit(dir: &Path, file: &str, msg: &str) {
    std::fs::write(dir.join(file), format!("{msg}\n")).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

fn rev(dir: &Path, rev: &str) -> String {
    let out = std::process::Command::new("git").current_dir(dir).args(["rev-parse", rev]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

async fn ticket(env: &Env) -> Value {
    call(&env.app, "GET", &format!("/api/tickets/{}", env.ticket), None).await.1
}

/// The ticket once `done` says so (polling, as CI runs on its own).
async fn wait_for(env: &Env, done: impl Fn(&Value) -> bool) -> Value {
    wait_until("the ticket", async || {
        let t = ticket(env).await;
        if done(&t) { Ok(t) } else { Err(t) }
    })
    .await
}

async fn events(env: &Env, kind: &str) -> Vec<Value> {
    let (_, v) = call(&env.app, "GET", &format!("/api/events?type={kind}"), None).await;
    v.as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn a_ticket_goes_through_local_ci_to_a_local_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), CI).await;
    // Nothing of its own yet: no change request.
    pdt_api::refresh_prs(&env.state).await;
    assert!(ticket(&env).await["attempt"]["pr"].is_null());

    // A commit without ok.txt: change request #1, and CI fails.
    commit(&env.worktree, "work.txt", "work");
    pdt_api::refresh_prs(&env.state).await;
    let t = wait_for(&env, |t| t["attempt"]["pr"]["ci"]["state"] == "failed").await;
    let pr = &t["attempt"]["pr"];
    assert_eq!(
        (pr["number"].as_u64(), pr["local"].as_bool(), pr["state"].as_str()),
        (Some(1), Some(true), Some("open"))
    );
    assert_eq!(pr["title"], format!("{}: Ship it", t["key"].as_str().unwrap()));
    let url = pr["ci"]["url"].as_str().unwrap().to_string();
    let path = url.split_once("/api/").map(|(_, p)| format!("/api/{p}")).unwrap();
    let (s, ct, body) = common::call_raw(&env.app, "GET", &path, vec![]).await;
    assert_eq!((s, ct.as_deref()), (StatusCode::OK, Some("text/plain; charset=utf-8")));
    let body = String::from_utf8_lossy(&body).into_owned();
    assert!(body.contains("$ git ls-files --error-unmatch ok.txt") && body.contains("exited with"), "{body}");
    // A run whose log is gone reads as empty; a run that never was is not found.
    let id: i64 = path.trim_end_matches("/log").rsplit('/').next().unwrap().parse().unwrap();
    std::fs::remove_file(env.state.data_dir.join("local-ci").join(format!("{id}.log"))).unwrap();
    let (s, _, body) = common::call_raw(&env.app, "GET", &path, vec![]).await;
    assert_eq!((s, body.len()), (StatusCode::OK, 0));
    let (s, _, _) = common::call_raw(&env.app, "GET", &format!("/api/local-ci/runs/{}/log", id + 100), vec![]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let failed = events(&env, "ci.run_failed").await;
    assert_eq!(failed.len(), 1, "{failed:#?}");
    let notice = failed[0]["payload"]["notice"]["text"].as_str().unwrap();
    assert!(notice.contains("change request #1") && notice.contains(&format!("curl -s {url}")), "{notice}");
    // PDT-242: the local CI command is a CI provider of its own.
    let p = &failed[0]["payload"];
    assert_eq!((p["provider"].as_str(), p["runs"][0]["provider"].as_str()), (Some("local"), Some("local")));
    assert_eq!(p["jobs"], json!(["git ls-files --error-unmatch ok.txt"]), "its job is the command");
    assert!(p["runs"][0]["id"].is_string(), "{p}");

    // Merge…: set to merge, it waits for CI.
    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"by": "user"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("queued")), "{r}");
    assert_eq!(ticket(&env).await["attempt"]["pr"]["autoMerge"], true);
    let base = rev(&env.repo, "main");

    // The fix: CI runs again on the new head, passes, and PDT merges it (a fast-forward) and ends the attempt.
    commit(&env.worktree, "ok.txt", "fix");
    let head = rev(&env.worktree, "HEAD");
    pdt_api::refresh_prs(&env.state).await;
    let t = wait_for(&env, |t| t["attempt"]["state"] == "merged").await;
    assert_eq!(rev(&env.repo, "main"), head, "main moved to the branch head");
    assert_ne!(head, base);
    assert!(env.repo.join("ok.txt").is_file(), "the checkout of main was moved too");
    assert_eq!(t["attempt"]["pr"]["state"], "merged", "{t:#}");
    // Merged in the read that saw CI pass (as auto-merge on a forge): the merge is the news.
    let merged = events(&env, "pr.merged").await;
    assert_eq!(merged.len(), 1, "{merged:#?}");
    assert!(merged[0]["payload"]["notice"]["text"].as_str().unwrap().contains("change request #1 (pdt-"));
}

/// PDT-405: a CI command the user hasn't approved doesn't run: CI waits, the Inbox says so, and once the user approves
/// it CI runs on the next read.
#[tokio::test]
async fn ci_waits_until_the_user_approves_its_command() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup_with(tmp.path(), CI, false).await;
    commit(&env.worktree, "ok.txt", "work");
    pdt_api::refresh_prs(&env.state).await;
    let t = wait_for(&env, |t| t["attempt"]["pr"]["number"] == 1).await;
    let waiting = notices(&env).await;
    let notice = waiting.iter().find(|i| i.body["key"] == "untrusted-commands").expect("a notice");
    assert!(notice.body["text"].as_str().unwrap().contains("git ls-files --error-unmatch ok.txt"), "{notice:?}");
    // Still waiting a read later: nothing ran.
    pdt_api::refresh_prs(&env.state).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let ci = ticket(&env).await["attempt"]["pr"]["ci"].clone();
    assert!(!["running", "passed", "failed"].contains(&ci["state"].as_str().unwrap_or("")), "{ci}");
    let pid = t["projectId"].as_str().unwrap();

    common::approve_commands(&env.app, pid, None).await;
    assert!(
        !notices(&env).await.iter().any(|i| i.body["key"] == "untrusted-commands"),
        "approving it closes the notice"
    );
    pdt_api::refresh_prs(&env.state).await;
    wait_for(&env, |t| t["attempt"]["pr"]["ci"]["state"] == "passed").await;
}

#[tokio::test]
async fn without_a_ci_command_merge_is_at_once_and_a_conflict_goes_back_to_the_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), "forge = \"local\"\n").await;
    commit(&env.worktree, "a.txt", "mine");
    commit(&env.repo, "a.txt", "theirs");
    pdt_api::refresh_prs(&env.state).await;
    let t = ticket(&env).await;
    let pr = &t["attempt"]["pr"];
    assert_eq!((pr["mergeable"].as_str(), pr["ci"].is_null()), (Some("conflicting"), true), "{t:#}");
    let conflict = events(&env, "pr.conflict").await;
    let text = conflict[0]["payload"]["notice"]["text"].as_str().unwrap();
    assert!(text.contains("change request #1") && text.contains("`git merge main`"), "{text}");
    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{r}");

    // The agent merges main in and resolves it: no CI, so Merge merges now, with a merge commit (main moved).
    git(&env.worktree, &["merge", "-q", "-s", "ours", "-m", "merge main", "main"]);
    pdt_api::refresh_prs(&env.state).await;
    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"by": "user"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("merged")), "{r}");
    assert_eq!(ticket(&env).await["attempt"]["state"], "merged");
    assert_eq!(rev(&env.repo, "main"), rev(&env.worktree, "HEAD"), "a fast-forward to the branch's merge");
}

/// `[push]` (PDT-245): the repository has a plain git remote, here a bare one in the temp folder.
const PUSH: &str = "forge = \"local\"\n\n[push]\nremote = \"origin\"\nbranches = true\n\n[ci]\ncommand = \"git ls-files \
                    --error-unmatch ok.txt\"\n";

/// A bare remote the repository's `main` is pushed to, as `origin`; the remote's folder.
fn add_remote(tmp: &Path, repo: &Path) -> PathBuf {
    let remote = tmp.join("remote.git");
    git(tmp, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(repo, &["push", "-q", "origin", "main"]);
    remote
}

/// Someone else pushes a commit to the remote's `main`; its sha.
fn push_from_elsewhere(tmp: &Path, remote: &Path, file: &str) -> String {
    let other = tmp.join(format!("other-{file}"));
    git(tmp, &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
    commit(&other, file, "elsewhere");
    git(&other, &["push", "-q", "origin", "main"]);
    rev(&other, "HEAD")
}

async fn merge(env: &Env) -> Value {
    let (s, r) =
        call(&env.app, "POST", &format!("/api/attempts/{}/merge", env.attempt), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    r
}

async fn notices(env: &Env) -> Vec<pdt_core::inbox::InboxItem> {
    env.state.store.open_inbox_items_of(pdt_core::inbox::InboxKind::Notice).await.unwrap()
}

#[tokio::test]
async fn with_a_remote_the_merge_is_pushed_and_so_are_ticket_branches() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), PUSH).await;
    let remote = add_remote(tmp.path(), &env.repo);
    commit(&env.worktree, "ok.txt", "work");
    let head = rev(&env.worktree, "HEAD");
    let branch = ticket(&env).await["attempt"]["branch"].as_str().unwrap().to_string();
    pdt_api::refresh_prs(&env.state).await;
    wait_for(&env, |t| t["attempt"]["pr"]["ci"]["state"] == "passed").await;
    // `branches = true`: the ticket branch went to the remote as it opened (in the background).
    for _ in 0..100 {
        if rev(&remote, &format!("refs/heads/{branch}")) == head {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(rev(&remote, &format!("refs/heads/{branch}")), head, "the branch was pushed");

    merge(&env).await;
    let t = wait_for(&env, |t| t["attempt"]["state"] == "merged").await;
    assert_eq!(t["attempt"]["pr"]["state"], "merged", "{t:#}");
    assert_eq!(rev(&env.repo, "main"), head, "a fast-forward");
    assert_eq!(rev(&remote, "main"), head, "main was pushed");
    assert!(env.repo.join("ok.txt").is_file(), "the checkout of main moved too");
    assert!(notices(&env).await.is_empty());
}

#[tokio::test]
async fn a_refused_push_catches_up_runs_ci_again_and_merges() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), PUSH).await;
    let remote = add_remote(tmp.path(), &env.repo);
    commit(&env.worktree, "ok.txt", "work");
    let head = rev(&env.worktree, "HEAD");
    pdt_api::refresh_prs(&env.state).await;
    wait_for(&env, |t| t["attempt"]["pr"]["ci"]["state"] == "passed").await;

    // The remote moves on before the merge: the push is refused, PDT fetches, catches main up and merges it into the
    // branch, CI runs on that, and the merge (a fast-forward now) is pushed.
    let theirs = push_from_elsewhere(tmp.path(), &remote, "theirs.txt");
    merge(&env).await;
    wait_for(&env, |t| t["attempt"]["state"] == "merged").await;
    let main = rev(&remote, "main");
    assert_eq!(rev(&env.repo, "main"), main, "local and remote main agree");
    assert_eq!(main, rev(&env.worktree, "HEAD"), "main is the branch with the remote's main merged in");
    for c in [&head, &theirs] {
        let ok = std::process::Command::new("git")
            .current_dir(&env.repo)
            .args(["merge-base", "--is-ancestor", c, &main])
            .status()
            .unwrap()
            .success();
        assert!(ok, "{c} is in main");
    }
    assert!(env.repo.join("theirs.txt").is_file() && env.repo.join("ok.txt").is_file());
    let runs = env.state.store.local_ci_runs(1).await.unwrap();
    assert_eq!(runs.len(), 2, "CI ran again on the caught-up head: {runs:?}");
    let (_, events) = call(&env.app, "GET", &format!("/api/tickets/{}/events", env.ticket), None).await;
    let noted = events["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "comment" && e["body"].as_str().is_some_and(|b| b.contains("had moved on")));
    assert!(noted, "{events:#}");
    assert!(notices(&env).await.is_empty());
}

#[tokio::test]
async fn a_push_that_cannot_catch_up_is_a_notice_and_waits_for_the_next_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), PUSH).await;
    let remote = add_remote(tmp.path(), &env.repo);
    commit(&env.worktree, "ok.txt", "work");
    pdt_api::refresh_prs(&env.state).await;
    wait_for(&env, |t| t["attempt"]["pr"]["ci"]["state"] == "passed").await;

    // Local main and the remote's diverged: PDT can't bring them together on its own.
    push_from_elsewhere(tmp.path(), &remote, "theirs.txt");
    git(&env.repo, &["switch", "-q", "--detach"]);
    commit(&env.repo, "local.txt", "local only");
    let local = rev(&env.repo, "HEAD");
    git(&env.repo, &["branch", "-f", "main", &local]);
    git(&env.repo, &["switch", "-q", "main"]);
    let before = rev(&remote, "main");
    merge(&env).await;
    let t = wait_for(&env, |t| t["attempt"]["pr"]["autoMerge"] == false).await;
    assert_eq!(t["attempt"]["state"], "active");
    assert_eq!((rev(&env.repo, "main"), rev(&remote, "main")), (local, before), "nothing moved");
    let n = notices(&env).await;
    assert_eq!(n.len(), 1, "{n:#?}");
    assert_eq!(n[0].attempt_id.as_deref(), Some(env.attempt.as_str()));
    let text = n[0].body["text"].as_str().unwrap();
    assert!(text.contains("diverged") && text.contains("Merge again"), "{text}");
}

#[tokio::test]
async fn a_push_with_no_access_is_a_notice() {
    let tmp = tempfile::tempdir().unwrap();
    let env = setup(tmp.path(), PUSH).await;
    // `origin` points at nothing.
    git(&env.repo, &["remote", "add", "origin", tmp.path().join("missing.git").to_str().unwrap()]);
    commit(&env.worktree, "ok.txt", "work");
    pdt_api::refresh_prs(&env.state).await;
    wait_for(&env, |t| t["attempt"]["pr"]["ci"]["state"] == "passed").await;
    let main = rev(&env.repo, "main");
    merge(&env).await;
    let t = wait_for(&env, |t| t["attempt"]["pr"]["autoMerge"] == false).await;
    assert_eq!((t["attempt"]["state"].as_str(), rev(&env.repo, "main")), (Some("active"), main), "not merged");
    let n = notices(&env).await;
    assert!(n[0].body["text"].as_str().unwrap().contains("could not push `main` to the remote `origin`"), "{n:#?}");
}
