//! The local forge for remote attempts (PDT-730): an owner (`studio`) whose project has no remote (`forge = "local"`,
//! a `[ci]` command) and a remote instance (`win-box`) on loopback. The remote's checkout is made from the owner's
//! base over the link; a `windows` ticket's branch there comes back to the owner, which opens its change request,
//! runs CI in its worktree there, and merges it into its own base once CI passes; the next attempt there starts
//! from that base.

use crate::common::wait::wait_until;
use crate::common::{self, call, call_as_ui, git};
use crate::remote_attempts::{Instance, instance};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const CONFIG: &str = "forge = \"local\"\n\n[ci]\ncommand = \"git ls-files --error-unmatch ok.txt\"\n";

fn commit(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), format!("{file}\n")).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", file]);
}

fn rev(dir: &Path, rev: &str) -> String {
    let out = std::process::Command::new("git").current_dir(dir).args(["rev-parse", rev]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Env {
    owner: Instance,
    remote: Instance,
    project: String,
    repo: PathBuf,
}

/// The owner's project (no origin, on the local forge, its CI command approved) with `windows` tickets placed on the
/// remote instance, which has no checkout of it yet.
async fn setup() -> Env {
    let owner = instance("studio", None).await;
    owner.state.link.set_no_join_heartbeat(true);
    let remote = instance("win-box", None).await;
    tokio::spawn(pdt_api::run_local_ci_watch(owner.state.clone()));
    let repo = owner.tmp.path().join("offline");
    common::init_repo(&repo);
    std::fs::create_dir_all(repo.join(".pdt")).unwrap();
    std::fs::write(repo.join(".pdt/project.toml"), CONFIG).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "config"]);
    let body = json!({"name": "Offline", "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&owner.app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    common::approve_commands(&owner.app, &project, None).await;

    let (_, v) = call(&owner.app, "POST", &format!("/api/projects/{project}/remote-instances/pairing"), None).await;
    let code = v["pairing"]["code"].as_str().unwrap().to_string();
    let join = json!({"ownerAddress": owner.url, "code": code, "address": remote.url});
    let (s, v) = call(&remote.app, "POST", "/api/remote-projects", Some(join)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    pdt_api::link::heartbeat_all(&owner.state).await;
    let (_, v) = call(&owner.app, "GET", &format!("/api/projects/{project}/remote-instances"), None).await;
    let instance_id = v["instances"][0]["instanceId"].as_str().unwrap().to_string();
    let rules = json!({"rules": [{"label": "Windows", "instance": instance_id}]});
    let (s, v) = call(&owner.app, "PUT", &format!("/api/projects/{project}/placement"), Some(rules)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    Env { owner, remote, project, repo }
}

/// Starts a `windows` ticket: (ticket, attempt, its worktree there).
async fn start(env: &Env, title: &str) -> (String, String, PathBuf) {
    let body = json!({"title": title, "labels": ["windows"]});
    let (s, t) =
        call_as_ui(&env.owner.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let ticket = t["id"].as_str().unwrap().to_string();
    let start = json!({"agent": "claude", "noSession": true});
    let (s, started) = call(&env.owner.app, "POST", &format!("/api/tickets/{ticket}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let attempt = started["attempt"]["id"].as_str().unwrap().to_string();
    let wt = started["attempt"]["worktreeId"].as_str().unwrap();
    let wt = env.owner.state.store.get_worktree(wt).await.unwrap().unwrap();
    assert_eq!(wt.instance.as_ref().map(|i| i.name.as_str()), Some("win-box"));
    let path = PathBuf::from(wt.path);
    assert!(path.starts_with(env.remote.tmp.path()), "{}", path.display());
    (ticket, attempt, path)
}

async fn ticket(env: &Env, id: &str) -> Value {
    call(&env.owner.app, "GET", &format!("/api/tickets/{id}"), None).await.1
}

async fn wait_for(env: &Env, id: &str, done: impl Fn(&Value) -> bool) -> Value {
    wait_until("the ticket", async || {
        let t = ticket(env, id).await;
        if done(&t) { Ok(t) } else { Err(t) }
    })
    .await
}

async fn ci_log(env: &Env, t: &Value) -> String {
    let url = t["attempt"]["pr"]["ci"]["url"].as_str().unwrap();
    let path = url.split_once("/api/").map(|(_, p)| format!("/api/{p}")).unwrap();
    let (s, _, body) = common::call_raw(&env.owner.app, "GET", &path, vec![]).await;
    assert_eq!(s, StatusCode::OK);
    String::from_utf8_lossy(&body).into_owned()
}

#[tokio::test]
async fn a_remote_attempt_goes_through_local_ci_there_to_a_merge_here() {
    let env = setup().await;
    // The remote's checkout comes from the owner's base: there is no origin to clone.
    let (id, attempt, worktree) = start(&env, "Ship it").await;
    assert!(worktree.join(".pdt/project.toml").is_file());
    assert_eq!(rev(&worktree, "HEAD"), rev(&env.repo, "main"));
    pdt_api::refresh_prs(&env.owner.state).await;
    assert!(ticket(&env, &id).await["attempt"]["pr"].is_null(), "nothing of its own yet");

    // A commit there without ok.txt: the branch comes here, change request #1 opens, and CI fails there.
    commit(&worktree, "work.txt");
    let branch = ticket(&env, &id).await["attempt"]["branch"].as_str().unwrap().to_string();
    pdt_api::refresh_prs(&env.owner.state).await;
    let t = wait_for(&env, &id, |t| t["attempt"]["pr"]["ci"]["state"] == "failed").await;
    assert_eq!(t["attempt"]["pr"]["number"], 1, "{t:#}");
    assert_eq!(rev(&env.repo, &branch), rev(&worktree, "HEAD"), "the branch is here too");
    let log = ci_log(&env, &t).await;
    assert!(log.contains("$ git ls-files --error-unmatch ok.txt") && log.contains("on win-box"), "{log}");
    assert!(log.contains("did not match any file") && log.contains("exited with 1"), "{log}");

    // Set to merge, the fix passes CI there and merges here: the owner's main is the branch's head.
    let (s, r) =
        call(&env.owner.app, "POST", &format!("/api/attempts/{attempt}/merge"), Some(json!({"by": "user"}))).await;
    assert_eq!((s, r["outcome"].as_str()), (StatusCode::OK, Some("queued")), "{r}");
    commit(&worktree, "ok.txt");
    let head = rev(&worktree, "HEAD");

    // While win-box can't be reached, the branch stays as last brought here: no CI, no merge.
    env.owner.state.link.set_offline(true);
    let before = rev(&env.repo, &branch);
    pdt_api::refresh_prs(&env.owner.state).await;
    let t = ticket(&env, &id).await;
    assert_eq!(
        (t["attempt"]["state"].as_str(), t["attempt"]["pr"]["ci"]["state"].as_str()),
        (Some("active"), Some("failed")),
        "{t:#}"
    );
    assert_eq!(rev(&env.repo, &branch), before);
    env.owner.state.link.set_offline(false);

    pdt_api::refresh_prs(&env.owner.state).await;
    let t = wait_for(&env, &id, |t| t["attempt"]["state"] == "merged").await;
    assert_eq!(t["attempt"]["pr"]["state"], "merged", "{t:#}");
    assert_eq!(rev(&env.repo, "main"), head);
    assert!(env.repo.join("ok.txt").is_file(), "the owner's checkout of main moved too");

    // The next attempt there starts from the merged base.
    let (_, _, next) = start(&env, "Next").await;
    assert_eq!(rev(&next, "HEAD"), head);
    assert!(next.join("ok.txt").is_file());
}
