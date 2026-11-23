//! Remote attempts (PDT-632): an owner (`studio`) and a remote instance (`win-box`) on loopback, paired for one
//! project. A `windows` ticket's attempt runs on the remote instance: its worktree is in the remote's checkout, its
//! agents (`fake-agent`) run there, their pdt-mcp calls are passed on to the owner, and their sessions are the owner's,
//! under the ticket and interactive, through the whole Pipeline. A dropped link loses nothing. (Terminals there:
//! `remote_terminals.rs`.)

use crate::common::wait::wait_until;
use crate::common::{self, call, call_as_ui, git, mark_pushed};
use axum::http::StatusCode;
use pdt_api::{AppState, router};
use pdt_core::pr::{Mergeable, PrState};
use pdt_forge::ChangeRequest;
use pdt_forge::testing::FakeForge;
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");
const ORIGIN: &str = "https://github.com/o/r.git";

pub(crate) struct Instance {
    pub(crate) app: axum::Router,
    pub(crate) state: AppState,
    pub(crate) url: String,
    pub(crate) tmp: tempfile::TempDir,
}

/// An instance named `name` with `fake-agent` as Claude, served on a free loopback port; with a fake GitHub when `gh`.
pub(crate) async fn instance(name: &str, gh: Option<&Arc<FakeForge>>) -> Instance {
    instance_with(name, gh, None).await
}

/// [`instance`], with its own update service when `update` is given.
pub(crate) async fn instance_with(
    name: &str,
    gh: Option<&Arc<FakeForge>>,
    update: Option<Arc<pdt_api::UpdateService>>,
) -> Instance {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    if let Some(u) = update {
        state.update = u;
    }
    state.platform.claude_program = FAKE.to_string();
    state.platform.codex_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.conversations.notice_quiet = Duration::ZERO;
    state.exec.set_poll_timing(Duration::from_millis(500), Duration::from_millis(100));
    if let Some(gh) = gh {
        state.forges = pdt_forge::ForgeRegistry::new(vec![Arc::new(pdt_forge::testing::fake_github(gh))]);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{}", state.listen_port);
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"instanceName": name}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    common::stuck_agents_off(&app).await;
    Instance { app, state, url, tmp }
}

/// A repository whose origin says GitHub (nothing is fetched or pushed: its proxy goes nowhere).
fn repo(dir: &Path) {
    common::init_repo(dir);
    git(dir, &["remote", "add", "origin", ORIGIN]);
    git(dir, &["config", "http.proxy", "http://127.0.0.1:1"]);
}

pub(crate) struct Pair {
    pub(crate) owner: Instance,
    pub(crate) remote: Instance,
    pub(crate) gh: Arc<FakeForge>,
    /// The owner's project.
    pub(crate) project: String,
    /// The remote instance's id of it.
    pub(crate) remote_project: String,
    /// The remote instance's id (as the owner names it).
    pub(crate) instance_id: String,
    /// Where the remote instance keeps the project's checkout.
    pub(crate) checkout: PathBuf,
}

/// Owner and remote paired for the owner's project, with `windows` tickets placed on the remote instance. Its
/// checkout is a clone of the owner's repository (a real one would be cloned from origin on first use).
pub(crate) async fn paired() -> Pair {
    paired_with(None).await
}

/// [`paired`], the remote instance with its own update service when `update` is given.
pub(crate) async fn paired_with(update: Option<Arc<pdt_api::UpdateService>>) -> Pair {
    let gh = Arc::new(FakeForge::default());
    let owner = instance("studio", Some(&gh)).await;
    let remote = instance_with("win-box", None, update).await;
    let main = owner.tmp.path().join("valley");
    repo(&main);
    let (s, p) =
        call(&owner.app, "POST", "/api/projects", Some(json!({"name": "Valley", "repoPath": main.to_string_lossy()})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let (_, v) = call(&owner.app, "POST", &format!("/api/projects/{project}/remote-instances/pairing"), None).await;
    let code = v["pairing"]["code"].as_str().unwrap().to_string();
    let join = json!({"ownerAddress": owner.url, "code": code, "address": remote.url});
    let (s, v) = call(&remote.app, "POST", "/api/remote-projects", Some(join)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let remote_project = v["project"]["id"].as_str().unwrap().to_string();

    let checkout = remote.tmp.path().join("checkouts").join("valley");
    std::fs::create_dir_all(checkout.parent().unwrap()).unwrap();
    git(remote.tmp.path(), &["clone", "-q", &main.to_string_lossy(), &checkout.to_string_lossy()]);
    git(&checkout, &["remote", "set-url", "origin", ORIGIN]);
    git(&checkout, &["config", "http.proxy", "http://127.0.0.1:1"]);
    git(&checkout, &["config", "user.name", "t"]);
    git(&checkout, &["config", "user.email", "t@t"]);
    let patch = json!({"checkoutPath": checkout.to_string_lossy()});
    let (s, v) = call(&remote.app, "PATCH", &format!("/api/remote-projects/{remote_project}"), Some(patch)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["checkoutPath"], checkout.to_string_lossy().as_ref());

    pdt_api::link::heartbeat_all(&owner.state).await;
    let (_, v) = call(&owner.app, "GET", &format!("/api/projects/{project}/remote-instances"), None).await;
    assert_eq!(v["instances"][0]["state"], "connected", "{v}");
    let instance_id = v["instances"][0]["instanceId"].as_str().unwrap().to_string();
    let rules = json!({"rules": [{"label": "Windows", "instance": instance_id}]});
    let (s, v) = call(&owner.app, "PUT", &format!("/api/projects/{project}/placement"), Some(rules)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["rules"][0]["label"], "windows");
    Pair { owner, remote, gh, project, remote_project, instance_id, checkout }
}

pub(crate) async fn ticket(p: &Pair, title: &str, labels: &[&str]) -> String {
    let body = json!({"title": title, "labels": labels});
    let (s, t) = call_as_ui(&p.owner.app, "POST", &format!("/api/projects/{}/tickets", p.project), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t["id"].as_str().unwrap().to_string()
}

pub(crate) async fn get_ticket(p: &Pair, id: &str) -> Value {
    call(&p.owner.app, "GET", &format!("/api/tickets/{id}"), None).await.1
}

pub(crate) async fn sessions(p: &Pair) -> Vec<Value> {
    let (_, v) = call(&p.owner.app, "GET", &format!("/api/projects/{}/sessions", p.project), None).await;
    v.as_array().unwrap().clone()
}

pub(crate) async fn worktrees(p: &Pair) -> Vec<Value> {
    let (_, v) = call(&p.owner.app, "GET", &format!("/api/projects/{}/worktrees", p.project), None).await;
    v.as_array().unwrap().clone()
}

pub(crate) async fn events(p: &Pair, session: &str) -> Vec<Value> {
    let (_, v) = call(&p.owner.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array().unwrap().clone()
}

pub(crate) async fn agent_says(p: &Pair, session: &str) -> Vec<String> {
    events(p, session)
        .await
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// Sends a message from the owner and waits for what the agent says after its first `n` things that starts with
/// `prefix`.
pub(crate) async fn said_after(p: &Pair, session: &str, n: usize, prefix: &str) -> String {
    wait_until(&format!("the agent to say {prefix}"), async || {
        let said = agent_says(p, session).await;
        said.get(n..).and_then(|s| s.iter().find(|m| m.starts_with(prefix)).cloned()).ok_or(said)
    })
    .await
}

pub(crate) async fn send(p: &Pair, session: &str, text: &str) {
    let url = format!("/api/sessions/{session}/messages");
    let (s, v) = call(&p.owner.app, "POST", &url, Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
}

fn pull_request(head: &str) -> ChangeRequest {
    ChangeRequest {
        number: 12,
        url: "https://github.com/o/r/pull/12".into(),
        title: "Change it".into(),
        state: PrState::Open,
        draft: false,
        head_sha: head.into(),
        base_branch: "main".into(),
        mergeable: Mergeable::Mergeable,
        behind: false,
        auto_merge: false,
        in_merge_queue: false,
        queue_position: None,
        queue_removal: None,
        created_at: None,
    }
}

/// The Pipeline for the project, every agent step's prompt the pdt-mcp call its fake persona makes, and enabled.
async fn pipeline(p: &Pair, outcomes: &[(&str, &str)]) {
    let app = &p.owner.app;
    let params = json!({"autoAcceptLabels": ["agentic"]});
    let body = json!({"params": params, "scope": "project", "projectId": p.project});
    let (s, w) = call(app, "POST", "/api/workflow-templates/builtin:pipeline/instantiate", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    let id = w["id"].as_str().unwrap().to_string();
    let mut doc = w["doc"].clone();
    for n in doc["nodes"].as_array_mut().unwrap() {
        if let Some((_, outcome)) = outcomes.iter().find(|(node, _)| n["id"] == *node) {
            n["config"]["prompt"] = json!(format!(
                r#"mcp: submit_outcome {{"outcome": "{outcome}", "summary": "{} says {outcome}"}}"#,
                n["id"].as_str().unwrap()
            ));
        }
    }
    let (s, v) = call(app, "PUT", &format!("/api/workflows/{id}"), Some(json!({"doc": doc, "note": "fake"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = call(app, "PATCH", &format!("/api/workflows/{id}"), Some(json!({"enabled": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
}

fn outcomes(events: &Value) -> Vec<String> {
    events["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "outcome")
        .map(|e| e["data"]["outcome"].as_str().unwrap().to_string())
        .collect()
}

fn head(dir: &Path) -> String {
    let out = std::process::Command::new("git").current_dir(dir).args(["rev-parse", "HEAD"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn a_windows_ticket_runs_the_whole_pipeline_on_the_remote_instance() {
    let p = paired().await;
    let owner = &p.owner;
    pipeline(&p, &[("code", "submitted"), ("review", "approved"), ("qa", "pass")]).await;
    let (s, v) = call(
        &owner.app,
        "PUT",
        &format!("/api/projects/{}/settings", p.project),
        Some(json!({"agentCredentials": null, "runOverrides": null, "requireGreenCi": false})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Started: its worktree is on win-box, in the remote's checkout; the owner records it with the instance.
    let t = ticket(&p, "Fix the Windows client", &["windows", "agentic"]).await;
    let start = json!({"agent": "claude", "noSession": true});
    let (s, started) = call(&owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
    let wt = worktrees(&p).await.into_iter().find(|w| w["branch"] == branch.as_str()).unwrap();
    let path = p.checkout.join(".worktrees").join(&branch);
    assert_eq!(wt["path"], path.to_string_lossy().as_ref(), "{wt}");
    assert_eq!((wt["instance"]["name"].as_str(), wt["missing"].as_bool()), (Some("win-box"), Some(false)), "{wt}");
    assert!(path.is_dir() && !owner.tmp.path().join("valley/.worktrees").join(&branch).exists());

    // What the Coder would have done there: a commit, pushed, with its pull request.
    std::fs::write(path.join("client.txt"), "built on windows\n").unwrap();
    git(&path, &["add", "."]);
    git(&path, &["commit", "-q", "-m", "client"]);
    mark_pushed(&path);
    p.gh.set_pr(&branch, Some(pull_request(&head(&path))));
    pdt_api::refresh_prs(&owner.state).await;

    // Code → Review → QA, each a conversation on win-box whose submit_outcome reaches the owner; then the merge.
    let merges = || p.gh.calls.lock().unwrap().iter().filter(|c| c.starts_with("merge o/r 12")).count();
    wait_until("auto-merge on", async || {
        pdt_api::run_workflows(&owner.state).await;
        merges() == 1
    })
    .await;
    let (_, events) = call(&owner.app, "GET", &format!("/api/tickets/{t}/events?limit=200"), None).await;
    assert_eq!(outcomes(&events), ["submitted", "approved", "pass"]);

    // Every step's session is the owner's, under the ticket, on win-box, in the remote worktree.
    let steps: Vec<Value> = sessions(&p).await.into_iter().filter(|s| s["ticket"]["id"] == t.as_str()).collect();
    assert_eq!(steps.len(), 3, "{steps:#?}");
    for s in &steps {
        assert_eq!(s["instance"]["name"], "win-box", "{s}");
        assert_eq!(s["instance"]["id"], p.instance_id.as_str());
        assert_eq!(s["cwd"], path.to_string_lossy().as_ref());
        let said = agent_says(&p, s["id"].as_str().unwrap()).await;
        assert!(said.iter().any(|m| m.starts_with("mcp submit_outcome")), "{said:?}");
    }
    // The remote instance ran them for the owner, and kept none of them as its own.
    let rows = p.remote.state.store.list_exec_sessions(None).await.unwrap();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows.iter().all(|r| r.ticket_key.is_some() && r.cwd == path.to_string_lossy()));
    let (_, projects) = call(&p.remote.app, "GET", "/api/projects", None).await;
    assert!(projects.as_array().unwrap().is_empty(), "{projects}");

    // Merged on GitHub: the ticket is done, and its worktree goes on win-box too.
    let mut pr = pull_request(&head(&path));
    pr.state = PrState::Merged;
    p.gh.set_pr(&branch, Some(pr));
    pdt_api::refresh_prs(&owner.state).await;
    wait_until("the ticket done", async || {
        pdt_api::check_merges(&owner.state).await;
        pdt_api::run_workflows(&owner.state).await;
        let v = get_ticket(&p, &t).await;
        if v["stage"] == "done" { Ok(()) } else { Err(json!({"stage": v["stage"], "pipeline": v["pipelineStage"], "attempt": v["attempts"][0], "agent": v["agent"]})) }
    })
    .await;
    wait_until("the remote worktree removed", async || !path.exists()).await;
    assert!(worktrees(&p).await.iter().all(|w| w["branch"] != branch.as_str()));
}

#[tokio::test]
async fn unlabelled_tickets_still_run_on_the_owner() {
    let p = paired().await;
    let t = ticket(&p, "Docs", &["docs"]).await;
    let start = json!({"agent": "claude", "noSession": true});
    let (s, started) = call(&p.owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap();
    let wt = worktrees(&p).await.into_iter().find(|w| w["branch"] == branch).unwrap();
    assert_eq!(wt["instance"], Value::Null, "{wt}");
    assert!(p.owner.tmp.path().join("valley/.worktrees").join(branch).is_dir());
    assert!(!p.checkout.join(".worktrees").join(branch).exists());

    // Runs on: the Start dialog's choice wins over the label rules; one that hasn't joined is refused.
    let t = ticket(&p, "Try it on win-box", &[]).await;
    let start = json!({"agent": "claude", "noSession": true, "runsOn": "nowhere"});
    let (s, v) = call(&p.owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("placement")), "{v}");
    let start = json!({"agent": "claude", "noSession": true, "runsOn": "win-box"});
    let (s, started) = call(&p.owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap();
    assert!(p.checkout.join(".worktrees").join(branch).is_dir());
}

#[tokio::test]
async fn a_remote_session_is_interactive_on_the_owner_and_a_dropped_link_loses_nothing() {
    let p = paired().await;
    let owner = &p.owner;
    let t = ticket(&p, "Port the client", &["windows"]).await;
    let start = json!({"agent": "claude", "permissionMode": "bypass"});
    let (s, started) = call(&owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let id = started["session"]["id"].as_str().unwrap().to_string();
    assert_eq!(started["session"]["instance"]["name"], "win-box");
    // It answers its first prompt, there; the owner stores what it says.
    wait_until("the first reply", async || !agent_says(&p, &id).await.is_empty()).await;
    let (_, listed) = call(&p.remote.app, "GET", "/api/remote-projects", None).await;
    let running = &listed[0]["running"];
    assert_eq!((running[0]["sessionId"].as_str(), running[0]["live"].as_bool()), (Some(id.as_str()), Some(true)));
    assert_eq!(running[0]["ticketKey"], get_ticket(&p, &t).await["key"]);

    // A relayed pdt-mcp call.
    let n = agent_says(&p, &id).await.len();
    send(&p, &id, "mcp: get_ticket {}").await;
    let said = said_after(&p, &id, n, "mcp get_ticket").await;
    assert!(said.contains("Port the client"), "{said}");

    // An asking tool asks in the conversation on the owner; the call waits there, and the user's Allow runs it.
    let n = agent_says(&p, &id).await.len();
    send(&p, &id, r#"mcp: change_request_open {"title": "Port the client"}"#).await;
    let req = wait_until("an open request", async || {
        let all = events(&p, &id).await;
        all.iter()
            .find(|e| e["event"]["kind"] == "request" && e["event"]["state"] == "open")
            .map(|e| e["event"].clone())
    })
    .await;
    assert_eq!(req["tool"], "mcp__pdt__change_request_open", "{req}");
    let url = format!("/api/sessions/{id}/requests/{}", req["id"].as_str().unwrap());
    let (s, v) = call_as_ui(&owner.app, "POST", &url, Some(json!({"answer": {"type": "approve"}}))).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    said_after(&p, &id, n, "mcp change_request_open").await;
    let req = events(&p, &id).await.into_iter().find(|e| e["event"]["id"] == req["id"]).unwrap();
    assert_eq!(req["event"]["state"], "answered", "{req}");

    // The remote instance can't reach the owner: the agent's call waits, and goes through once it can.
    p.remote.state.link.set_offline(true);
    let n = agent_says(&p, &id).await.len();
    send(&p, &id, "mcp: get_ticket {}").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(agent_says(&p, &id).await.len(), n, "nothing came back meanwhile");
    p.remote.state.link.set_offline(false);
    let said = said_after(&p, &id, n, "mcp get_ticket").await;
    assert!(said.contains("Port the client"), "{said}");

    // The whole link drops mid-turn: the agent works on there, and everything it said arrives once it is back.
    let n = agent_says(&p, &id).await.len();
    send(&p, &id, "slow").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    owner.state.link.set_offline(true);
    p.remote.state.link.set_offline(true);
    tokio::time::sleep(Duration::from_secs(2)).await;
    owner.state.link.set_offline(false);
    p.remote.state.link.set_offline(false);
    said_after(&p, &id, n, "Finished the slow job.").await;
    wait_until("the slow turn's end", async || {
        let turns: Vec<Value> = events(&p, &id).await.into_iter().filter(|e| e["event"]["kind"] == "turn").collect();
        turns.last().is_some_and(|e| e["event"]["phase"] == "ended")
    })
    .await;

    // Interrupt and stop from the owner reach the process there.
    send(&p, &id, "slow").await;
    wait_until("the slow turn", async || {
        let turns: Vec<Value> = events(&p, &id).await.into_iter().filter(|e| e["event"]["kind"] == "turn").collect();
        turns.last().is_some_and(|e| e["event"]["phase"] == "started")
    })
    .await;
    let (s, v) = call(&owner.app, "POST", &format!("/api/sessions/{id}/interrupt"), None).await;
    assert!(s.is_success(), "{v}");
    let n = agent_says(&p, &id).await.len();
    send(&p, &id, "mcp: get_ticket {}").await;
    said_after(&p, &id, n, "mcp get_ticket").await;
    let (s, v) = call(&owner.app, "POST", &format!("/api/sessions/{id}/kill"), None).await;
    assert!(s.is_success(), "{v}");
    wait_until("the process there stopped", async || {
        let (_, listed) = call(&p.remote.app, "GET", "/api/remote-projects", None).await;
        listed[0]["running"].as_array().unwrap().iter().all(|r| r["live"] == false)
    })
    .await;
    // Sent again, it opens there again and carries on.
    let n = agent_says(&p, &id).await.len();
    send(&p, &id, "mcp: get_ticket {}").await;
    said_after(&p, &id, n, "mcp get_ticket").await;
    let _ = &p.remote_project;
}

#[tokio::test]
async fn the_remote_instance_clones_the_checkout_on_first_use() {
    let owner = instance("studio", None).await;
    let remote = instance("win-box", None).await;
    // Origin is a repository on disk, so the remote instance can clone it.
    let origin = owner.tmp.path().join("origin");
    common::init_repo(&origin);
    let main = owner.tmp.path().join("valley");
    git(owner.tmp.path(), &["clone", "-q", &origin.to_string_lossy(), &main.to_string_lossy()]);
    let (s, p) =
        call(&owner.app, "POST", "/api/projects", Some(json!({"name": "Valley", "repoPath": main.to_string_lossy()})))
            .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    let project = p["id"].as_str().unwrap().to_string();
    let (_, v) = call(&owner.app, "POST", &format!("/api/projects/{project}/remote-instances/pairing"), None).await;
    let join = json!({"ownerAddress": owner.url, "code": v["pairing"]["code"], "address": remote.url});
    let (s, joined) = call(&remote.app, "POST", "/api/remote-projects", Some(join)).await;
    assert_eq!(s, StatusCode::CREATED, "{joined}");
    let checkout = PathBuf::from(joined["checkoutPath"].as_str().unwrap());
    assert!(checkout.starts_with(remote.tmp.path()) && !checkout.exists(), "{joined}");
    pdt_api::link::heartbeat_all(&owner.state).await;

    let (s, t) =
        call_as_ui(&owner.app, "POST", &format!("/api/projects/{project}/tickets"), Some(json!({"title": "Go"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let start = json!({"agent": "claude", "noSession": true, "runsOn": "win-box"});
    let (s, started) =
        call(&owner.app, "POST", &format!("/api/tickets/{}/start", t["id"].as_str().unwrap()), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap();
    assert!(checkout.join(".git").exists(), "cloned");
    assert!(checkout.join(".worktrees").join(branch).is_dir());
}

/// Pickup per instance (PDT-636): win-box is a slot pool of its own beside the owner's; a ticket placed on it waits in
/// To do while it is offline (the Board says why, and the owner notifies once it has been gone a while), and starts
/// once it is back; a ticket whose repository needs no instance has says so; and each end's cap sizes the pool.
#[tokio::test]
async fn pickup_fills_each_instance_on_its_own_and_waits_while_one_is_offline() {
    let p = paired().await;
    let owner = &p.owner;
    // Starts hand over to a workflow, so they make no agent session.
    let doc = json!({"name": "Takes over", "nodes": [
        {"id": "started", "kind": "attempt.started", "version": 1},
        {"id": "end", "kind": "end", "version": 1}],
        "edges": [{"from": "started", "to": "end"}]});
    let body = json!({"doc": doc, "scope": "project", "projectId": p.project, "enabled": true});
    let (s, v) = call(&owner.app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let settings = json!({"agentCredentials": null, "runOverrides": null, "autoStart": "todo", "startDelaySeconds": 0});
    let (s, v) = call(&owner.app, "PUT", &format!("/api/projects/{}/settings", p.project), Some(settings)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The repository says `plan9` tickets need an OS nobody has.
    let main = owner.tmp.path().join("valley");
    std::fs::create_dir_all(main.join(".pdt")).unwrap();
    let toml = "[[placement]]\nwhen = { labels = [\"plan9\"] }\nneeds = { os = \"plan9\" }\n";
    std::fs::write(main.join(".pdt/project.toml"), toml).unwrap();

    let stage = async |id: &str| get_ticket(&p, id).await["stage"].as_str().unwrap().to_string();
    let todo = async |title: &str, labels: &[&str]| {
        let body = json!({"title": title, "labels": labels, "stage": "todo"});
        let (s, t) = call_as_ui(&owner.app, "POST", &format!("/api/projects/{}/tickets", p.project), Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        t["id"].as_str().unwrap().to_string()
    };
    let home = todo("Docs", &[]).await;
    let w1 = todo("Client one", &["windows"]).await;
    let w2 = todo("Client two", &["windows"]).await;
    let nine = todo("Port to Plan 9", &["plan9"]).await;

    // One run fills both pools: the owner's cap of one and win-box's (one when neither end sets it).
    common::pick_up(&owner.state).await;
    let started = [stage(&home).await, stage(&w1).await, stage(&w2).await, stage(&nine).await];
    assert_eq!(started.iter().filter(|s| *s == "in_progress").count(), 2, "{started:?}");
    assert_eq!(started[0], "in_progress", "the owner's pool");
    let (on_box, waiting) = if started[1] == "in_progress" { (&w1, &w2) } else { (&w2, &w1) };
    let branch = get_ticket(&p, on_box).await["attempts"][0]["branch"].as_str().unwrap().to_string();
    let wt = worktrees(&p).await.into_iter().find(|w| w["branch"] == branch.as_str()).unwrap();
    assert_eq!(wt["instance"]["name"], "win-box", "{wt}");
    assert!(wt["instance"]["lastSeenAt"].is_number(), "{wt}");
    assert_eq!(get_ticket(&p, &nine).await["pickupHold"], "no instance can run this (needs os=plan9)");

    // win-box goes offline (gone for three hours) while the project's cap for it rises to two: the other windows
    // ticket waits for it, and says so.
    owner.state.link.set_offline(true);
    let (_, v) = call(&owner.app, "GET", &format!("/api/projects/{}/remote-instances", p.project), None).await;
    let link = v["instances"][0]["linkId"].as_str().unwrap().to_string();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let gone = now - 3 * 60 * 60 * 1000;
    let facts = pdt_core::link::InstanceFacts::default();
    let old = pdt_store::Heard::Answer {
        name: "win-box",
        version: v["instances"][0]["version"].as_str().unwrap(),
        protocol: pdt_core::link::PROTOCOL,
        facts: &facts,
        paused: false,
        secrets: &[],
        cap: None,
        at: gone,
    };
    owner.state.store.record_heartbeat(&link, old).await.unwrap();
    let url = format!("/api/projects/{}/remote-instances/{link}", p.project);
    let (s, v) = call(&owner.app, "PATCH", &url, Some(json!({"cap": 2}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["instances"][0]["cap"].as_u64(), v["instances"][0]["poolCap"].as_u64()), (Some(2), Some(2)));
    assert_eq!(v["instances"][0]["state"], "offline");
    common::pick_up(&owner.state).await;
    assert_eq!(stage(waiting).await, "todo");
    assert_eq!(get_ticket(&p, waiting).await["pickupHold"], "waiting for win-box (offline)");

    // Gone that long with work waiting: the owner says so, once.
    let offline_notes = async || {
        let (_, v) = call(&owner.app, "GET", "/api/notifications?section=updates", None).await;
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["dedupeKey"] == format!("instance-offline:{link}").as_str() && n["clearedAt"].is_null())
            .cloned()
            .collect::<Vec<Value>>()
    };
    pdt_api::link::heartbeat_all(&owner.state).await;
    pdt_api::link::heartbeat_all(&owner.state).await;
    let notes = offline_notes().await;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0]["title"], "win-box is offline with work waiting");
    assert_eq!(notes[0]["target"]["id"], waiting.as_str());

    // Back: the notification clears, and the ticket starts on win-box.
    owner.state.link.set_offline(false);
    pdt_api::link::heartbeat_all(&owner.state).await;
    assert!(offline_notes().await.is_empty());
    common::pick_up(&owner.state).await;
    assert_eq!(stage(waiting).await, "in_progress");
    let (_, events) = call(&owner.app, "GET", "/api/events?type=slot.freed&limit=50", None).await;
    assert!(events.as_array().unwrap().iter().any(|e| e["payload"]["reason"] == "instance"), "{events}");

    // win-box's own cap for the project is the lower one.
    let patch = json!({"cap": 1});
    let (s, v) = call(&p.remote.app, "PATCH", &format!("/api/remote-projects/{}", p.remote_project), Some(patch)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["link"]["cap"], 1);
    pdt_api::link::heartbeat_all(&owner.state).await;
    let (_, v) = call(&owner.app, "GET", &format!("/api/projects/{}/remote-instances", p.project), None).await;
    assert_eq!((v["instances"][0]["remoteCap"].as_u64(), v["instances"][0]["poolCap"].as_u64()), (Some(1), Some(1)));
}
