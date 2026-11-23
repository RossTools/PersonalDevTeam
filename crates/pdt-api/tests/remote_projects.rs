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
    instance_with(name, |_| {}).await
}

/// An instance named `name`, with `set` applied to its state before it serves.
async fn instance_with(name: &str, set: impl FnOnce(&mut AppState)) -> Instance {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = common::app_state(store, tmp.path());
    state.platform.login_path = None;
    set(&mut state);
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
    assert!(i["lastSeenAt"].is_number());

    let (_, listed) = call(&remote.app, "GET", "/api/remote-projects", None).await;
    assert!(listed[0]["link"]["lastContactAt"].is_number(), "the owner's heartbeat reached it");

    // Paused on the remote instance: the owner hears it with the next heartbeat.
    let (s, v) = call(&remote.app, "PATCH", &format!("/api/remote-projects/{rp}"), Some(json!({"paused": true}))).await;
    assert_eq!((s, &v["state"]), (StatusCode::OK, &json!("paused")), "{v}");
    let (s, v) = call(&owner.app, "POST", &format!("/api/projects/{p}/remote-instances/check"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["instances"][0]["state"], "paused", "{v}");
    let facts = &v["instances"][0]["facts"];
    assert!(facts["agents"].as_array().unwrap().iter().any(|a| a["agent"] == "claude"), "Check now checked: {v}");
    assert!(facts["agentsCheckedAt"].is_number(), "{v}");

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
    let (s, v) = signed_heartbeat(&remote, &key, "no-such-link", &p, &[]).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::FORBIDDEN, Some("link_revoked")), "{v}");

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
    // Its answer to the first message, before asking anything else.
    common::pm::wait_said(&env, &pm, 1).await;
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
    let (s, v) = call(&owner.app, "DELETE", &format!("/api/projects/{p}/remote-instances/{link}"), None).await;
    assert_eq!((s, &v), (StatusCode::OK, &json!({"told": true})));
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
    let (s, v) = call(&remote.app, "DELETE", &format!("/api/remote-projects/{rp}"), None).await;
    assert_eq!((s, &v), (StatusCode::OK, &json!({"told": true})));
    assert!(instances(&owner, &p).await["instances"].as_array().unwrap().is_empty(), "the owner was told");
}

/// The remote instance's link to the owner, made quiet: the owner last reached it long enough ago to count as offline.
async fn gone_quiet(remote: &Instance, rp: &str) -> String {
    let (_, l) =
        remote.state.store.list_remote_projects().await.unwrap().into_iter().find(|(p, _)| p.id == rp).unwrap();
    remote.state.store.owner_contact(&l.link_id, now_ms() - pdt_core::link::OFFLINE_AFTER_MS - 1, None).await.unwrap();
    l.link_id
}

async fn remote_projects(remote: &Instance) -> Value {
    let (s, v) = call(&remote.app, "GET", "/api/remote-projects", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[tokio::test]
async fn removing_an_instance_that_is_offline_says_so_and_it_finds_out_later() {
    let (owner, remote, p, rp) = paired().await;
    let link = instances(&owner, &p).await["instances"][0]["linkId"].as_str().unwrap().to_string();
    owner.state.link.set_offline(true);
    let (s, v) = call(&owner.app, "DELETE", &format!("/api/projects/{p}/remote-instances/{link}"), None).await;
    assert_eq!((s, &v), (StatusCode::OK, &json!({"told": false})), "win-box couldn't be told");
    assert!(instances(&owner, &p).await["instances"].as_array().unwrap().is_empty());
    assert!(owner.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "the owner's key is gone");
    owner.state.link.set_offline(false);

    // win-box still lists the project; once the owner's heartbeats stop, it asks, and learns it was removed.
    assert_eq!(remote_projects(&remote).await[0]["state"], "connected");
    pdt_api::link::check_owners(&remote.state).await;
    assert_eq!(remote_projects(&remote).await[0]["state"], "connected", "a link heard from lately isn't asked about");
    gone_quiet(&remote, &rp).await;
    assert_eq!(remote_projects(&remote).await[0]["state"], "offline");
    pdt_api::link::check_owners(&remote.state).await;
    assert_eq!(remote_projects(&remote).await[0]["state"], "removed", "{}", remote_projects(&remote).await);

    // Leaving it then has nothing left to tell the owner.
    let (s, v) = call(&remote.app, "DELETE", &format!("/api/remote-projects/{rp}"), None).await;
    assert_eq!((s, &v), (StatusCode::OK, &json!({"told": true})));
    assert_eq!(remote_projects(&remote).await, json!([]));
    assert!(remote.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "win-box's key is gone");
}

#[tokio::test]
async fn an_owner_still_linked_but_unreachable_is_only_offline() {
    let (_owner, remote, _p, rp) = paired().await;
    gone_quiet(&remote, &rp).await;
    remote.state.link.set_offline(true);
    pdt_api::link::check_owners(&remote.state).await;
    assert_eq!(remote_projects(&remote).await[0]["state"], "offline");
    remote.state.link.set_offline(false);
    pdt_api::link::check_owners(&remote.state).await;
    assert_eq!(remote_projects(&remote).await[0]["state"], "offline", "studio still has the link");
}

#[tokio::test]
async fn leaving_while_the_owner_is_offline_says_so_and_its_heartbeats_say_it_left() {
    let (owner, remote, p, rp) = paired().await;
    let link = instances(&owner, &p).await["instances"][0]["linkId"].as_str().unwrap().to_string();
    remote.state.link.set_offline(true);
    let (s, v) = call(&remote.app, "DELETE", &format!("/api/remote-projects/{rp}"), None).await;
    assert_eq!((s, &v), (StatusCode::OK, &json!({"told": false})), "studio couldn't be told");
    assert_eq!(remote_projects(&remote).await, json!([]));
    assert!(remote.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "win-box's key is gone");

    pdt_api::link::heartbeat_all(&owner.state).await;
    let i = &instances(&owner, &p).await["instances"][0];
    assert_eq!(i["lastError"], "win-box left the project; remove it here too", "{i}");
    let (s, v) = call(&owner.app, "DELETE", &format!("/api/projects/{p}/remote-instances/{link}"), None).await;
    assert_eq!((s, &v), (StatusCode::OK, &json!({"told": true})), "win-box no longer has it: nothing to tell");
}

#[tokio::test]
async fn deleting_a_project_revokes_its_links() {
    let (owner, remote, p, _) = paired().await;
    let link = instances(&owner, &p).await["instances"][0]["linkId"].as_str().unwrap().to_string();
    let (s, v) = call(&owner.app, "DELETE", &format!("/api/projects/{p}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert_eq!(remote_projects(&remote).await, json!([]), "win-box was told");
    assert!(owner.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "the owner's key is gone");
    assert!(remote.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "and win-box's");
}

#[tokio::test]
async fn deleting_a_project_while_its_instance_is_offline_still_drops_the_key() {
    let (owner, remote, p, rp) = paired().await;
    let link = instances(&owner, &p).await["instances"][0]["linkId"].as_str().unwrap().to_string();
    owner.state.link.set_offline(true);
    let (s, v) = call(&owner.app, "DELETE", &format!("/api/projects/{p}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert!(owner.state.secrets.get(&format!("project-link:{link}")).unwrap().is_none(), "the owner's key is gone");
    owner.state.link.set_offline(false);
    gone_quiet(&remote, &rp).await;
    pdt_api::link::check_owners(&remote.state).await;
    assert_eq!(remote_projects(&remote).await[0]["state"], "removed");
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

/// A fake `claude` whose login check takes `delay` seconds and answers `logged_in`; [`set_claude`] changes both.
#[cfg(unix)]
fn fake_claude(dir: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("claude");
    let d = dir.to_string_lossy();
    std::fs::write(&p, format!("#!/bin/sh\nsleep \"$(cat '{d}/delay')\"\ncat '{d}/out'\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.to_string_lossy().into()
}

#[cfg(unix)]
fn set_claude(dir: &std::path::Path, delay: u32, logged_in: bool) {
    std::fs::write(dir.join("delay"), delay.to_string()).unwrap();
    std::fs::write(dir.join("out"), format!(r#"{{"loggedIn": {logged_in}}}"#)).unwrap();
}

/// The owner's view of win-box's claude: (installed, logged in), or None while win-box hasn't checked its agents.
#[cfg(unix)]
fn claude_fact(view: &Value) -> Option<(bool, bool)> {
    let facts = &view["instances"][0]["facts"];
    if facts["agentsCheckedAt"].is_null() {
        assert_eq!(facts["agents"], json!([]), "unchecked agents are unknown: {view}");
        return None;
    }
    let a = facts["agents"].as_array().unwrap().iter().find(|a| a["agent"] == "claude").unwrap();
    Some((a["installed"].as_bool().unwrap(), a["loggedIn"].as_bool().unwrap()))
}

/// A heartbeat answers at once from the last agent checks (none yet, right after a start: they run in the
/// background), so a slow agent CLI never makes the instance look offline; the checks run again once they are old,
/// and Check now has them run again before it answers, so a login since shows.
#[cfg(unix)]
#[tokio::test]
async fn heartbeats_answer_from_the_last_agent_checks_and_check_now_runs_them_again() {
    let fake = tempfile::tempdir().unwrap();
    set_claude(fake.path(), 3, false);
    let claude = fake_claude(fake.path());
    let owner = instance("studio").await;
    let remote = instance_with("win-box", |st| st.platform.claude_program = claude).await;
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    let c = code(&owner, &p).await;
    assert_eq!(join(&remote, &owner, &c).await.0, StatusCode::CREATED);

    pdt_api::link::heartbeat_all(&owner.state).await;
    let v = instances(&owner, &p).await;
    assert_eq!(v["instances"][0]["state"], "connected", "{v}");
    assert_eq!(claude_fact(&v), None, "the heartbeat didn't wait for the checks");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let first = loop {
        pdt_api::link::heartbeat_all(&owner.state).await;
        if let Some(f) = claude_fact(&instances(&owner, &p).await) {
            break f;
        }
        assert!(std::time::Instant::now() < deadline, "the background checks never finished");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    };
    assert_eq!(first, (true, false));

    // Logged in since: a heartbeat still answers from the checks it made (they aren't old yet); Check now asks again.
    set_claude(fake.path(), 0, true);
    pdt_api::link::heartbeat_all(&owner.state).await;
    assert_eq!(claude_fact(&instances(&owner, &p).await), Some((true, false)));
    let (s, v) = call(&owner.app, "POST", &format!("/api/projects/{p}/remote-instances/check"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(claude_fact(&v), Some((true, true)), "{v}");
}

/// Another instance at the other end of a link that answers `path` with `answer(path)` and anything else with `{}`:
/// a peer sending whatever it likes.
async fn fake_peer(answer: impl Fn(&str) -> Option<String> + Clone + Send + Sync + 'static) -> String {
    let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let answer = answer.clone();
        async move {
            let body = answer(uri.path()).unwrap_or_else(|| "{}".into());
            ([("content-type", "application/json")], body)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await });
    url
}

fn public_key() -> String {
    let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes())
}

fn hello(name: &str, version: &str) -> Value {
    json!({"instanceId": "peer", "name": name, "version": version, "protocol": pdt_core::link::PROTOCOL})
}

#[tokio::test]
async fn joining_takes_from_the_owner_only_what_pairing_would_take() {
    let remote = instance("win-box").await;
    remote.state.link.set_no_join_heartbeat(true);
    let join_owner = |owner: Value, key: &str, branch: &str| {
        let reply = json!({
            "linkId": uuid::Uuid::now_v7().to_string(),
            "project": {"id": uuid::Uuid::now_v7().to_string(), "key": key, "name": "Valley app", "baseBranch": branch},
            "owner": owner,
            "publicKey": public_key(),
        })
        .to_string();
        let remote_url = remote.url.clone();
        let app = remote.app.clone();
        async move {
            let url = fake_peer(move |p| (p == pdt_api::link::PAIR_PATH).then(|| reply.clone())).await;
            let body = json!({"ownerAddress": url, "code": "ABCD-EFGH", "address": remote_url});
            call(&app, "POST", "/api/remote-projects", Some(body)).await
        }
    };

    // A key in lower case is this instance's key upper-cased; one that isn't a key at all is suggested from the name.
    let (s, v) = join_owner(hello("studio", "1.0.0"), "valley", "main").await;
    assert_eq!((s, &v["project"]["key"]), (StatusCode::CREATED, &json!("VALLEY")), "{v}");
    let (s, v) = join_owner(hello("studio", "1.0.0"), "a-b", "main").await;
    assert_eq!((s, &v["project"]["key"]), (StatusCode::CREATED, &json!("VA")), "{v}");
    let (s, v) = join_owner(hello("studio", "1.0.0"), "Valley", "main").await;
    assert_eq!((s, &v["project"]["key"]), (StatusCode::CREATED, &json!("VA2")), "taken in another case: {v}");

    // A name, version or branch pairing would refuse leaves no remote project behind.
    let before = remote_projects(&remote).await.as_array().unwrap().len();
    for (owner, branch) in [
        (hello("evil name\u{1b}[2J", "1.0.0"), "main"),
        (hello("studio", "1.0 <b>"), "main"),
        (hello("studio", "1.0.0"), "-main"),
    ] {
        let (s, v) = join_owner(owner, "WEB", branch).await;
        assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
    }
    assert_eq!(remote_projects(&remote).await.as_array().unwrap().len(), before);
}

#[tokio::test]
async fn heartbeats_from_a_peer_are_checked_and_capped() {
    let owner = instance("studio").await;
    owner.state.link.set_no_join_heartbeat(true);
    let p = project(&owner, "valley", "Valley", ORIGIN).await;
    let answer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sent = answer.clone();
    let url =
        fake_peer(move |path| (path == pdt_api::link::HEARTBEAT_PATH).then(|| sent.lock().unwrap().clone())).await;
    let pair = |version: &str| json!({"code": "", "instance": hello("win-box", version), "url": url, "publicKey": public_key()});

    let mut req = pair("1.0 beta");
    req["code"] = json!(code(&owner, &p).await);
    let (s, v) = call(&owner.app, "POST", pdt_api::link::PAIR_PATH, Some(req)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a version pairing can't read: {v}");
    let mut req = pair("1.0.0");
    req["code"] = json!(code(&owner, &p).await);
    let (s, v) = call(&owner.app, "POST", pdt_api::link::PAIR_PATH, Some(req)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let reply =
        |name: &str, version: &str| json!({"instance": hello(name, version), "facts": {}, "paused": false}).to_string();
    *answer.lock().unwrap() = reply(" win-box ", "1.0.1");
    pdt_api::link::heartbeat_all(&owner.state).await;
    let i = &instances(&owner, &p).await["instances"][0];
    assert_eq!(
        (&i["name"], &i["version"], &i["state"]),
        (&json!("win-box"), &json!("1.0.1"), &json!("connected")),
        "{i}"
    );

    for (name, version, problem) in [("evil name\u{1b}[2J", "1.0.2", "name"), ("win-box", "1.0.2\n<b>", "version")] {
        *answer.lock().unwrap() = reply(name, version);
        pdt_api::link::heartbeat_all(&owner.state).await;
        let i = &instances(&owner, &p).await["instances"][0];
        assert_eq!((&i["name"], &i["version"]), (&json!("win-box"), &json!("1.0.1")), "nothing stored: {i}");
        assert!(i["lastError"].as_str().unwrap().contains(problem), "{i}");
    }

    // An answer too large to be a heartbeat isn't read to its end.
    *answer.lock().unwrap() = format!("\"{}\"", "x".repeat(2 * 1024 * 1024));
    pdt_api::link::heartbeat_all(&owner.state).await;
    let i = &instances(&owner, &p).await["instances"][0];
    assert!(i["lastError"].as_str().unwrap().contains("larger than"), "{i}");
}
