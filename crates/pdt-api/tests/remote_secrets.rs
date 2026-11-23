//! Secrets for sessions on a remote instance, and updating instances together (PDT-637): an owner (`studio`) and a
//! remote instance (`win-box`) on loopback, as in `remote_attempts.rs`. A remote session's `run_with_secrets` and
//! `write_secret_file` run on win-box with the owner's value forwarded for the call, never kept or logged there; a
//! secret win-box keeps for the project serves it there, never crosses the link, and wins on a name clash. The owner's
//! update step names an instance on another protocol and asks it to update on the owner's channel.

use crate::common::call;
use crate::common::wait::wait_until;
use crate::remote_attempts::{Pair, agent_says, paired, paired_with, said_after, send, ticket};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const OWNER_VALUE: &str = "npm_owner-Zq8wX2-forwarded";
const OWNER_CERT: &str = "cert_owner-Lk3pQ9-never-sent";
const LOCAL_CERT: &str = "cert_winbox-Hj7tR4-stays-there";

/// Everything both instances log while the guard lives (this test's thread: `#[tokio::test]` runs every task on it).
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Logs {
    /// The captured logs, and what keeps the capture on: drop them at the test's end.
    fn capture() -> (Self, (tracing::subscriber::DefaultGuard, tracing::Dispatch)) {
        let logs = Logs::default();
        let writer = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .finish();
        // Callsite interest is cached process-wide (as in pdt-api `loops.rs`'s test): with one dispatcher registered,
        // a test on another thread that reaches a callsite first with no subscriber caches "never". A second
        // dispatcher alive for the whole test makes every thread ask all of them, this one included.
        let second = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let guard = tracing::subscriber::set_default(subscriber);
        // And callsites registered before the two dispatchers were.
        tracing::callsite::rebuild_interest_cache();
        (logs.clone(), (guard, second))
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// Whether any of the instance's database files hold `needle`.
fn store_holds(dir: &Path, needle: &str) -> bool {
    ["pdt.db", "pdt.db-wal", "pdt.db-shm"]
        .iter()
        .any(|f| std::fs::read(dir.join(f)).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes())))
}

/// A `windows` ticket's Coder on win-box, permissions bypassed (the secret tools ask nothing), with its first reply.
async fn remote_session(p: &Pair) -> (String, PathBuf) {
    let t = ticket(p, "Publish the client", &["windows"]).await;
    let start = json!({"agent": "claude", "permissionMode": "bypass"});
    let (s, started) = call(&p.owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    assert_eq!(started["session"]["instance"]["name"], "win-box");
    let id = started["session"]["id"].as_str().unwrap().to_string();
    wait_until("the first reply", async || !agent_says(p, &id).await.is_empty()).await;
    let cwd = PathBuf::from(started["session"]["cwd"].as_str().unwrap());
    (id, cwd)
}

/// The agent calls `tool` with `args`; what it got back.
async fn tool(p: &Pair, session: &str, tool: &str, args: Value) -> String {
    let n = agent_says(p, session).await.len();
    send(p, session, &format!("mcp: {tool} {args}")).await;
    said_after(p, session, n, &format!("mcp {tool}")).await
}

async fn add_secret(app: &axum::Router, name: &str, value: &str, project: &str) -> Value {
    let body = json!({"name": name, "value": value, "projects": [project]});
    let (s, v) = call(app, "POST", "/api/secrets", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

#[tokio::test]
async fn an_owners_secret_is_forwarded_for_one_call_and_never_kept_or_logged_on_the_remote_instance() {
    let (logs, _guard) = Logs::capture();
    let p = paired().await;
    let secret = add_secret(&p.owner.app, "NPM_TOKEN", OWNER_VALUE, &p.project).await;
    let (id, cwd) = remote_session(&p).await;
    assert!(cwd.starts_with(&p.checkout), "it runs in win-box's checkout: {}", cwd.display());

    // It runs on win-box, with the owner's value; what it printed comes back redacted.
    let echo = if cfg!(windows) { "echo %TOK% & cd" } else { "echo \"$TOK\"; pwd" };
    let said = tool(&p, &id, "run_with_secrets", json!({"command": echo, "env": {"TOK": "secret:NPM_TOKEN"}})).await;
    assert!(said.contains("[secret:NPM_TOKEN]") && !said.contains(OWNER_VALUE), "{said}");
    assert!(said.contains("\"exitCode\": 0"), "{said}");
    let leaf = cwd.file_name().unwrap().to_string_lossy().into_owned();
    assert!(said.contains(&leaf), "in the session's folder there: {said}");
    if cfg!(unix) {
        let encoded = json!({"command": "printf %s \"$TOK\" | base64", "env": {"TOK": "secret:NPM_TOKEN"}});
        let said = tool(&p, &id, "run_with_secrets", encoded).await;
        assert!(said.contains("[secret:NPM_TOKEN]"), "encoded, it is replaced too: {said}");
    }

    // A file there with it filled in; nothing of it comes back.
    let write = json!({"path": ".npmrc", "template": "//registry/:_authToken={{secret.NPM_TOKEN}}\n"});
    let said = tool(&p, &id, "write_secret_file", write).await;
    assert!(said.contains("\"bytes\"") && !said.contains(OWNER_VALUE), "{said}");
    assert_eq!(std::fs::read_to_string(cwd.join(".npmrc")).unwrap(), format!("//registry/:_authToken={OWNER_VALUE}\n"));
    // Outside the session's folder, refused there as here.
    let out = json!({"path": "../../elsewhere.env", "template": "{{secret.NPM_TOKEN}}"});
    assert!(tool(&p, &id, "write_secret_file", out).await.contains("failed"));
    let missing = json!({"command": "echo", "env": {"X": "secret:NOPE"}});
    assert!(tool(&p, &id, "run_with_secrets", missing).await.contains("no secret is called NOPE"));

    // Audited on the owner, with the instance it went to.
    let sid = secret["id"].as_str().unwrap();
    let (_, uses) = call(&p.owner.app, "GET", &format!("/api/secrets/{sid}/uses"), None).await;
    let uses = uses.as_array().unwrap();
    let runs = if cfg!(unix) { 2 } else { 1 };
    assert_eq!(uses.len(), runs + 1, "{uses:?}");
    assert!(uses.iter().all(|u| u["instance"] == "win-box" && u["sessionId"] == id.as_str()), "{uses:?}");
    assert_eq!(uses[0]["how"], "write_secret_file");

    // win-box never kept it: not in its secrets, its database or anything either instance logged.
    let (_, listed) = call(&p.remote.app, "GET", "/api/secrets", None).await;
    assert_eq!(listed, json!([]));
    assert!(!store_holds(p.remote.tmp.path(), OWNER_VALUE), "win-box's store holds the forwarded value");
    assert!(!store_holds(p.owner.tmp.path(), OWNER_VALUE), "the owner's store holds it unredacted");
    let logged = logs.text();
    assert!(logged.contains("a remote session's secret tool ran"), "the logs were captured: {logged}");
    assert!(!logged.contains(OWNER_VALUE), "a log holds the forwarded value");
    assert_eq!(p.remote.state.store.redactor().string(OWNER_VALUE), OWNER_VALUE, "win-box's redactor never knew it");
}

#[tokio::test]
async fn a_secret_the_remote_instance_keeps_serves_it_there_never_crosses_the_link_and_wins_a_clash() {
    let p = paired().await;
    let owner_cert = add_secret(&p.owner.app, "CERT", OWNER_CERT, &p.project).await;
    add_secret(&p.owner.app, "NPM_TOKEN", OWNER_VALUE, &p.project).await;
    // Kept on win-box for the remote project (its Secrets, the remote project's page), and one for none in particular,
    // which stays win-box's own.
    let local = add_secret(&p.remote.app, "cert", LOCAL_CERT, &p.remote_project).await;
    let body = json!({"name": "OTHER", "value": "other-value-for-winbox"});
    assert_eq!(call(&p.remote.app, "POST", "/api/secrets", Some(body)).await.0, StatusCode::CREATED);
    pdt_api::link::heartbeat_all(&p.owner.state).await;

    // The owner lists them by name, on win-box.
    let (_, sets) = call(&p.owner.app, "GET", "/api/secrets/remote", None).await;
    assert_eq!(sets.as_array().unwrap().len(), 1, "{sets}");
    assert_eq!(sets[0]["instanceName"], "win-box");
    assert_eq!(sets[0]["projectId"], p.project.as_str());
    assert_eq!(sets[0]["secrets"], json!([{"name": "cert", "lastUsedAt": null, "lastUsedBy": null}]));
    assert!(!sets.to_string().contains(LOCAL_CERT));

    let (id, cwd) = remote_session(&p).await;
    let said = tool(&p, &id, "list_secrets", json!({})).await;
    assert!(said.contains("kept on win-box") && said.contains("NPM_TOKEN") && !said.contains("OTHER"), "{said}");

    // The clash: win-box's own wins, and with the owner's forwarded alongside.
    let write = json!({"path": "conf/app.env", "template": "CERT={{secret.CERT}}\nNPM={{secret.NPM_TOKEN}}\n"});
    let said = tool(&p, &id, "write_secret_file", write).await;
    assert!(said.contains("\"bytes\""), "{said}");
    let written = std::fs::read_to_string(cwd.join("conf").join("app.env")).unwrap();
    assert_eq!(written, format!("CERT={LOCAL_CERT}\nNPM={OWNER_VALUE}\n"));
    let echo = if cfg!(windows) { "echo %C%" } else { "echo \"$C\"" };
    let said = tool(&p, &id, "run_with_secrets", json!({"command": echo, "env": {"C": "secret:CERT"}})).await;
    assert!(said.contains("[secret:CERT]") && !said.contains(LOCAL_CERT), "{said}");
    // The one for no project in particular isn't the remote project's.
    let said = tool(&p, &id, "run_with_secrets", json!({"command": echo, "env": {"C": "secret:OTHER"}})).await;
    assert!(said.contains("no secret is called OTHER"), "{said}");

    // Its value never reached the owner; the owner's of the same name was never sent, so never used.
    assert!(!store_holds(p.owner.tmp.path(), LOCAL_CERT), "the owner's store holds win-box's value");
    let oid = owner_cert["id"].as_str().unwrap();
    let (_, uses) = call(&p.owner.app, "GET", &format!("/api/secrets/{oid}/uses"), None).await;
    assert_eq!(uses, json!([]), "the owner's CERT was never forwarded");
    // Uses of win-box's own: on win-box, and reported to the owner by name.
    let lid = local["id"].as_str().unwrap();
    let (_, there) = call(&p.remote.app, "GET", &format!("/api/secrets/{lid}/uses"), None).await;
    let how: Vec<&str> = there.as_array().unwrap().iter().map(|u| u["how"].as_str().unwrap()).collect();
    assert_eq!(how, ["run_with_secrets", "write_secret_file"], "{there}");
    assert_eq!(there[0]["sessionId"], id.as_str());
    let (_, sets) = call(&p.owner.app, "GET", "/api/secrets/remote", None).await;
    let cert = &sets[0]["secrets"][0];
    assert!(cert["lastUsedAt"].is_number() && cert["lastUsedBy"].is_string(), "{sets}");
}

#[tokio::test]
#[cfg_attr(not(any(windows, target_os = "macos")), ignore = "releases only ship installers for macOS and Windows")]
async fn the_owner_asks_an_instance_on_another_protocol_to_update_on_its_channel() {
    // win-box under pdt-host, behind a release; studio on the stable channel, win-box's own set to nightly.
    let (base, _fake) = crate::update::fake_github().await;
    let svc = pdt_api::UpdateService::new(&base, "o/r", true, "0.1.0-local");
    let p = paired_with(Some(Arc::new(svc))).await;
    let (s, v) = call(&p.remote.app, "PUT", "/api/host/settings", Some(json!({"updateChannel": "nightly"}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // win-box speaks an older protocol: the link refuses it, and the update step says to update it.
    p.remote.state.link.set_protocol(pdt_core::link::PROTOCOL - 1);
    pdt_api::link::heartbeat_all(&p.owner.state).await;
    let (_, step) = call(&p.owner.app, "GET", "/api/host/update/instances", None).await;
    assert_eq!((step["name"].as_str(), step["channel"].as_str()), (Some("studio"), Some("stable")), "{step}");
    let win = &step["instances"][0];
    assert_eq!(win["name"], "win-box");
    assert_eq!(win["state"], "incompatible");
    let needed = win["needed"].as_str().unwrap();
    assert!(needed.starts_with("update win-box to v") && needed.ends_with("to link it with studio"), "{win}");

    // Asked to update, it checks on the owner's channel and installs what it found.
    let url = format!("/api/host/update/instances/{}", p.instance_id);
    let (s, v) = call(&p.owner.app, "POST", &url, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({"name": "win-box", "outcome": "installing", "version": "0.1.7", "channel": "stable"}));
    tokio::time::timeout(Duration::from_secs(10), p.remote.state.shutdown.notified())
        .await
        .expect("win-box handed the update to pdt-host and stopped");
}

#[tokio::test]
async fn an_instance_that_cant_update_itself_says_why() {
    let p = paired().await;
    pdt_api::link::heartbeat_all(&p.owner.state).await;
    let (_, step) = call(&p.owner.app, "GET", "/api/host/update/instances", None).await;
    assert_eq!(step["instances"][0]["state"], "connected", "{step}");
    assert_eq!(step["instances"][0]["needed"], Value::Null, "same version, same protocol");
    let url = format!("/api/host/update/instances/{}", p.instance_id);
    let (s, v) = call(&p.owner.app, "POST", &url, None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("updates_disabled")), "{v}");
    assert!(v["message"].as_str().unwrap().starts_with("win-box: "), "{v}");
    let (s, _) = call(&p.owner.app, "POST", "/api/host/update/instances/nobody", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
