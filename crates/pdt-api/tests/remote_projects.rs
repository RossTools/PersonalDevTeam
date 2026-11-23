//! Remote projects (PDT-631): two pdtd instances on loopback, an owner (`studio`) and a remote instance (`win-box`),
//! pair for one project, exchange heartbeats and facts, refuse what isn't signed by the link's other end, and keep the
//! remote project out of the remote instance's own lists. Tailscale identities are the headers `tailscale serve` adds.

use crate::common::{self, call, call_with, git};
use axum::http::StatusCode;
use base64::Engine;
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

struct Instance {
    app: axum::Router,
    state: AppState,
    url: String,
    tmp: tempfile::TempDir,
}

/// An instance named `name`, served on a free loopback port.
async fn instance(name: &str) -> Instance {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.platform.login_path = None;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{}", state.listen_port);
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"instanceName": name}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    Instance { app, state, url, tmp }
}

/// A repository with one commit and `origin` at `origin`, added to `i` as a project.
async fn project(i: &Instance, dir: &str, name: &str, origin: &str) -> String {
    let repo = i.tmp.path().join(dir);
    common::init_repo(&repo);
    git(&repo, &["remote", "add", "origin", origin]);
    let (s, p) =
        call(&i.app, "POST", "/api/projects", Some(json!({"name": name, "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    p["id"].as_str().unwrap().to_string()
}

const ORIGIN: &str = "git@github.com:someone/valley.git";

async fn code(owner: &Instance, project: &str) -> String {
    let (s, v) = call(&owner.app, "POST", &format!("/api/projects/{project}/remote-instances/pairing"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["pairing"]["code"].as_str().unwrap().to_string()
}

async fn join(remote: &Instance, owner: &Instance, code: &str) -> (StatusCode, Value) {
    let body = json!({"ownerAddress": owner.url, "code": code, "address": remote.url});
    call(&remote.app, "POST", "/api/remote-projects", Some(body)).await
}

/// Owner and remote paired for the owner's project: (owner, remote, owner's project id, remote's project id).
async fn paired() -> (Instance, Instance, String, String) {
    let owner = instance("studio").await;
    let remote = instance("win-box").await;
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    let c = code(&owner, &p).await;
    let (s, v) = join(&remote, &owner, &c).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let remote_project = v["project"]["id"].as_str().unwrap().to_string();
    (owner, remote, p, remote_project)
}

async fn instances(owner: &Instance, project: &str) -> Value {
    let (s, v) = call(&owner.app, "GET", &format!("/api/projects/{project}/remote-instances"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[tokio::test]
async fn two_instances_pair_for_one_project_and_heartbeats_carry_facts() {
    let (owner, remote, p, rp) = paired().await;

    let (_, listed) = call(&remote.app, "GET", "/api/remote-projects", None).await;
    let r = &listed[0];
    assert_eq!(r["project"]["name"], "Valley");
    assert_eq!(r["project"]["kind"], "remote");
    assert_eq!(r["link"]["ownerName"], "studio");
    assert_eq!(r["link"]["repoUrl"], ORIGIN);
    assert_eq!(r["state"], "connected");
    assert_eq!(r["openUrl"], format!("{}/tickets?view=board", owner.url));

    pdt_api::link::heartbeat_all(&owner.state).await;
    let v = instances(&owner, &p).await;
    assert_eq!(v["name"], "studio");
    let i = &v["instances"][0];
    assert_eq!(i["name"], "win-box");
    assert_eq!(i["state"], "connected", "{i}");
    assert_eq!(i["version"], pdt_core::BUILD_VERSION);
    assert_eq!(i["facts"]["os"], std::env::consts::OS);
    assert!(i["facts"]["cpus"].as_u64().unwrap() >= 1, "{i}");
    assert!(i["facts"]["agents"].as_array().unwrap().iter().any(|a| a["agent"] == "claude"), "{i}");
    assert!(i["lastSeenAt"].is_number());

    let (_, listed) = call(&remote.app, "GET", "/api/remote-projects", None).await;
    assert!(listed[0]["link"]["lastContactAt"].is_number(), "the owner's heartbeat reached it");

    // Paused on the remote instance: the owner hears it with the next heartbeat.
    let (s, v) = call(&remote.app, "PATCH", &format!("/api/remote-projects/{rp}"), Some(json!({"paused": true}))).await;
    assert_eq!((s, &v["state"]), (StatusCode::OK, &json!("paused")), "{v}");
    let (s, v) = call(&owner.app, "POST", &format!("/api/projects/{p}/remote-instances/check"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["instances"][0]["state"], "paused", "{v}");

    // A code works once.
    let (s, v) = call(&owner.app, "POST", &format!("/api/projects/{p}/remote-instances/pairing"), None).await;
    assert_eq!(s, StatusCode::OK);
    let c = v["pairing"]["code"].as_str().unwrap().to_string();
    let other = instance("alex-laptop").await;
    assert_eq!(join(&other, &owner, &c).await.0, StatusCode::CREATED);
    let (s, v) = join(&other, &owner, &c).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("pairing_code")), "{v}");
    assert_eq!(instances(&owner, &p).await["instances"].as_array().unwrap().len(), 2, "one project, two instances");
}

#[tokio::test]
async fn a_wrong_code_is_refused() {
    let owner = instance("studio").await;
    let remote = instance("win-box").await;
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    code(&owner, &p).await;
    let (s, v) = join(&remote, &owner, "ZZZZ-ZZZZ").await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("pairing_code")), "{v}");
    let (s, v) = join(&remote, &owner, "nope").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(instances(&owner, &p).await["instances"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn another_protocol_is_refused_with_which_instance_to_update() {
    let owner = instance("studio").await;
    let remote = instance("win-box").await;
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    remote.state.link.set_protocol(pdt_core::link::PROTOCOL + 1);
    let c = code(&owner, &p).await;
    let (s, v) = join(&remote, &owner, &c).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("link_incompatible")), "{v}");
    let msg = v["message"].as_str().unwrap();
    assert!(msg.starts_with("update studio to v"), "{msg}");

    // Paired, then the remote instance moves on to another protocol: the owner's heartbeat says who must update.
    remote.state.link.set_protocol(pdt_core::link::PROTOCOL);
    let c = code(&owner, &p).await;
    assert_eq!(join(&remote, &owner, &c).await.0, StatusCode::CREATED);
    remote.state.link.set_protocol(pdt_core::link::PROTOCOL + 1);
    pdt_api::link::heartbeat_all(&owner.state).await;
    let i = &instances(&owner, &p).await["instances"][0];
    assert_eq!(i["state"], "incompatible", "{i}");
    assert!(i["problem"].as_str().unwrap().starts_with("update studio to v"), "{i}");

    // Older than the owner: the remote instance is the one to update.
    remote.state.link.set_protocol(0);
    owner.state.link.set_protocol(pdt_core::link::PROTOCOL);
    pdt_api::link::heartbeat_all(&owner.state).await;
    let i = &instances(&owner, &p).await["instances"][0];
    assert_eq!(i["state"], "incompatible", "{i}");
    assert!(i["problem"].as_str().unwrap().starts_with("update win-box to v"), "{i}");
}

/// The owner's private key for its project's first link, from its (in-memory) keychain.
fn owner_key(owner: &Instance, link: &str) -> ed25519_dalek::SigningKey {
    let raw = owner.state.secrets.get(&format!("project-link:{link}")).unwrap().unwrap();
    let bytes: [u8; 32] = base64::engine::general_purpose::STANDARD.decode(raw).unwrap().try_into().unwrap();
    ed25519_dalek::SigningKey::from_bytes(&bytes)
}

async fn signed_heartbeat(
    remote: &Instance,
    key: &ed25519_dalek::SigningKey,
    link: &str,
    project: &str,
    extra: &[(&str, &str)],
) -> (StatusCode, Value) {
    let body = json!({"owner": {"instanceId": "x", "name": "studio", "version": "1", "protocol": 1}});
    let bytes = serde_json::to_vec(&body).unwrap();
    let now = now_ms();
    let headers = pdt_api::link::sign_headers(key, pdt_api::link::HEARTBEAT_PATH, link, project, &bytes, 1, now);
    let mut all: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    all.extend_from_slice(extra);
    call_with(&remote.app, "POST", pdt_api::link::HEARTBEAT_PATH, Some(body), &all).await
}

#[tokio::test]
async fn a_wrong_key_the_wrong_project_or_an_unpaired_caller_is_refused() {
    let (owner, remote, p, _) = paired().await;
    let link = instances(&owner, &p).await["instances"][0]["linkId"].as_str().unwrap().to_string();
    let key = owner_key(&owner, &link);
    let refused = |r: (StatusCode, Value)| {
        assert_eq!((r.0, r.1["code"].as_str()), (StatusCode::FORBIDDEN, Some("link_refused")), "{}", r.1);
        r.1["message"].as_str().unwrap().to_string()
    };

    let (s, v) = signed_heartbeat(&remote, &key, &link, &p, &[]).await;
    assert_eq!(s, StatusCode::OK, "the owner's own request: {v}");
    assert_eq!(v["instance"]["name"], "win-box");

    let (s, v) = call(&remote.app, "POST", pdt_api::link::HEARTBEAT_PATH, Some(json!({}))).await;
    assert!(refused((s, v)).contains("not a signed"));
    let stranger = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    assert!(refused(signed_heartbeat(&remote, &stranger, &link, &p, &[]).await).contains("signature"));
    assert!(refused(signed_heartbeat(&remote, &key, &link, "another-project", &[]).await).contains("isn't for"));
    assert!(refused(signed_heartbeat(&remote, &key, "no-such-link", &p, &[]).await).contains("isn't paired"));

    // The same signed request twice: the second is a replay.
    let body = json!({});
    let bytes = serde_json::to_vec(&body).unwrap();
    let headers = pdt_api::link::sign_headers(&key, pdt_api::link::HEARTBEAT_PATH, &link, &p, &bytes, 1, now_ms());
    let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (s, _) = call_with(&remote.app, "POST", pdt_api::link::HEARTBEAT_PATH, Some(body.clone()), &h).await;
    assert_eq!(s, StatusCode::OK);
    let again = call_with(&remote.app, "POST", pdt_api::link::HEARTBEAT_PATH, Some(body.clone()), &h).await;
    assert!(refused(again).contains("already taken"));
    // Signed long ago.
    let old = now_ms() - 60 * 60 * 1000;
    let headers = pdt_api::link::sign_headers(&key, pdt_api::link::HEARTBEAT_PATH, &link, &p, &bytes, 1, old);
    let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    assert!(
        refused(call_with(&remote.app, "POST", pdt_api::link::HEARTBEAT_PATH, Some(body), &h).await).contains("clock")
    );

    // The owner's leave route takes only the remote instance's key.
    let headers = pdt_api::link::sign_headers(&key, pdt_api::link::LEAVE_PATH, &link, &p, b"{}", 1, now_ms());
    let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let r = call_with(&owner.app, "POST", pdt_api::link::LEAVE_PATH, Some(json!({})), &h).await;
    assert!(refused(r).contains("signature"), "the owner's own key isn't the remote instance's");
    assert_eq!(instances(&owner, &p).await["instances"].as_array().unwrap().len(), 1);

    // Over Tailscale, the caller must also be the host owner, signature or not.
    let (s, v) = call(&remote.app, "PUT", "/api/host/settings", Some(json!({"ownerLogin": "me@example.com"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    remote.state.refresh_owner().await;
    let someone = [("tailscale-user-login", "someone@example.com"), ("x-forwarded-for", "100.64.0.9")];
    let (s, v) = signed_heartbeat(&remote, &key, &link, &p, &someone).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden_identity")), "{v}");
    let tagged = [("x-forwarded-for", "100.64.0.9")];
    assert_eq!(signed_heartbeat(&remote, &key, &link, &p, &tagged).await.0, StatusCode::FORBIDDEN);
    let me = [("tailscale-user-login", "me@example.com"), ("x-forwarded-for", "100.64.0.9")];
    assert_eq!(signed_heartbeat(&remote, &key, &link, &p, &me).await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_remote_project_is_invisible_to_the_remote_instances_own_lists() {
    let (_owner, remote, _p, rp) = paired().await;
    let mine = project(&remote, "own", "Own work", "git@github.com:someone/own.git").await;

    let (_, projects) = call(&remote.app, "GET", "/api/projects", None).await;
    let ids: Vec<&str> = projects.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![mine.as_str()], "only its own project");

    for (method, uri, body) in [
        ("POST", format!("/api/projects/{rp}/tickets"), Some(json!({"title": "x", "stage": "todo"}))),
        ("GET", format!("/api/projects/{rp}"), None),
        ("GET", format!("/api/projects/{rp}/settings"), None),
        ("DELETE", format!("/api/projects/{rp}"), None),
    ] {
        let (s, v) = call(&remote.app, method, &uri, body).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("remote_project")), "{method} {uri}: {v}");
    }
    let (_, tickets) = call(&remote.app, "GET", "/api/tickets", None).await;
    assert!(!tickets.to_string().contains(&rp), "{tickets}");
    let (_, inbox) = call(&remote.app, "GET", "/api/inbox", None).await;
    assert!(!inbox.to_string().contains("Valley"), "{inbox}");
    // Pickup starts nothing for it (it has no tickets here), and its pass looks only at this instance's projects.
    pdt_api::watch_pickup(&remote.state).await;
}

#[tokio::test]
async fn the_remote_instances_pm_never_sees_the_remote_project() {
    let env = common::pm::setup().await;
    let owner = instance("studio").await;
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    let c = code(&owner, &p).await;
    let url = format!("http://127.0.0.1:{}", env.state.listen_port);
    let body = json!({"ownerAddress": owner.url, "code": c, "address": url});
    let (s, v) = call(&env.app, "POST", "/api/remote-projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let pm = common::pm::start_pm(&env, "hello").await;
    let said = common::pm::ask(&env, &pm, "mcp: get_overview {}").await;
    assert!(said.contains("PX") && !said.contains("Valley"), "{said}");
    let said = common::pm::ask(&env, &pm, "mcp: get_overview {\"project\": \"Valley\"}").await;
    assert!(said.contains("no project is called Valley"), "{said}");
    let briefing = std::fs::read_to_string(env.state.data_dir.join("sessions").join(&pm).join("briefing.md")).unwrap();
    assert!(briefing.contains("PX") && !briefing.contains("Valley"), "{briefing}");
}

#[tokio::test]
async fn an_instance_has_a_project_local_or_remote_never_both() {
    let owner = instance("studio").await;
    let remote = instance("win-box").await;
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    // The remote instance already has the repository as its own project.
    let own = project(&remote, "valley", "Valley here", "https://github.com/someone/valley").await;
    let c = code(&owner, &p).await;
    let (s, v) = join(&remote, &owner, &c).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("remote_project")), "{v}");
    assert!(v["message"].as_str().unwrap().contains("Valley here"), "{v}");
    assert!(instances(&owner, &p).await["instances"].as_array().unwrap().is_empty(), "the owner was told");

    // Without it, joining works; then the repository can't be added as its own project.
    let (s, _) = call(&remote.app, "DELETE", &format!("/api/projects/{own}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let c = code(&owner, &p).await;
    assert_eq!(join(&remote, &owner, &c).await.0, StatusCode::CREATED);
    let repo = remote.tmp.path().join("valley");
    let (s, v) =
        call(&remote.app, "POST", "/api/projects", Some(json!({"name": "Again", "repoPath": repo.to_string_lossy()})))
            .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("remote_project")), "{v}");

    // Pairing an instance with itself is refused.
    let c = code(&owner, &p).await;
    let (s, v) = join(&owner, &owner, &c).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("remote_project")), "{v}");
}

#[tokio::test]
async fn removing_at_either_end_revokes_the_link_at_both() {
    let (owner, remote, p, rp) = paired().await;
    let link = instances(&owner, &p).await["instances"][0]["linkId"].as_str().unwrap().to_string();
    let (s, _) = call(&owner.app, "DELETE", &format!("/api/projects/{p}/remote-instances/{link}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, listed) = call(&remote.app, "GET", "/api/remote-projects", None).await;
    assert_eq!(listed, json!([]), "the remote instance was told");
    assert!(remote.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "its key is gone");
    assert!(owner.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "and the owner's");
    let _ = rp;

    // Paired again, then left from the remote instance.
    let c = code(&owner, &p).await;
    let (s, v) = join(&remote, &owner, &c).await;
    assert_eq!(s, StatusCode::CREATED);
    let rp = v["project"]["id"].as_str().unwrap();
    let (s, _) = call(&remote.app, "DELETE", &format!("/api/remote-projects/{rp}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(instances(&owner, &p).await["instances"].as_array().unwrap().is_empty(), "the owner was told");
}

#[tokio::test]
async fn joining_needs_this_instances_address() {
    let owner = instance("studio").await;
    let remote = instance("win-box").await;
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    let c = code(&owner, &p).await;
    let body = json!({"ownerAddress": owner.url, "code": c});
    let (s, v) = call(&remote.app, "POST", "/api/remote-projects", Some(body)).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("remote_access_off")), "{v}");
    let body = json!({"ownerAddress": "http://studio.example.ts.net", "code": c, "address": remote.url});
    let (s, v) = call(&remote.app, "POST", "/api/remote-projects", Some(body)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "plain http off this machine: {v}");
}

#[tokio::test]
async fn the_instance_name_shows_in_health_and_settings() {
    let i = instance("studio").await;
    let (_, h) = call(&i.app, "GET", "/api/health", None).await;
    assert_eq!(h["instanceName"], "studio");
    let (_, s) = call(&i.app, "GET", "/api/host/settings", None).await;
    assert_eq!(s["settings"]["instanceName"], "studio", "{s}");
    let (s, v) = call(&i.app, "PUT", "/api/host/settings", Some(json!({"instanceName": "two words"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}
