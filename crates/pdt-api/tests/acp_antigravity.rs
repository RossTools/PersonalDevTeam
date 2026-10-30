//! The built-in Antigravity profile (PDT-239) end to end without the network or Google's server: Settings → Agents →
//! Install reads a recorded ACP registry and an archive from a fake source (checked against the registry's sha256),
//! Sign in asks the installed program to `authenticate`, and a conversation runs on it. The archive's program is a
//! script that starts `fake-agent --plain acp`.

mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use sha2::Digest;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");
const REGISTRY: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const ARCHIVE: &str = "https://dl.google.com/agy-extensions/releases/test/agy-acp-server-1.4.5.zip";

/// Answers from a map of URLs; anything else fails, as it would offline.
struct Fake(HashMap<String, Vec<u8>>);

#[async_trait::async_trait]
impl pdt_api::toolchains::ToolchainSource for Fake {
    async fn get(&self, url: &str, _limit: u64) -> Result<Vec<u8>, String> {
        self.0.get(url).cloned().ok_or_else(|| format!("no answer for {url}"))
    }
}

fn zip_of(name: &str, body: &[u8]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    w.start_file(name, zip::write::SimpleFileOptions::default().unix_permissions(0o755)).unwrap();
    w.write_all(body).unwrap();
    w.finish().unwrap().into_inner()
}

/// The registry's Antigravity entry as published (1.4.5), with this platform's build pointing at `ARCHIVE`.
fn registry(sha: &str) -> Vec<u8> {
    let platform = pdt_core::acp_registry::platform_key().unwrap();
    let mut binary = serde_json::Map::new();
    binary.insert(platform, json!({"archive": ARCHIVE, "cmd": "./agy_acp_server.par", "sha256": sha}));
    serde_json::to_vec(&json!({"version": "1.0.0", "agents": [{
        "id": "antigravity-acp", "name": "Google Antigravity", "version": "1.4.5",
        "authors": ["Google LLC"], "license": "proprietary", "distribution": {"binary": binary},
    }]}))
    .unwrap()
}

fn source(zip: &[u8], sha: &str) -> Arc<Fake> {
    Arc::new(Fake(HashMap::from([(REGISTRY.into(), registry(sha)), (ARCHIVE.into(), zip.to_vec())])))
}

fn antigravity(v: &Value) -> Value {
    v["acpAgents"].as_array().unwrap().iter().find(|a| a["id"] == "antigravity").cloned().unwrap()
}

async fn events(app: &axum::Router, id: &str) -> Vec<Value> {
    let (_, v) = call(app, "GET", &format!("/api/sessions/{id}/events?after=0&limit=1000"), None).await;
    v.as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn antigravity_installs_verified_signs_in_and_runs_a_conversation() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "--allow-empty", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.login_path = None;
    // Never the real ~/.gemini.
    state.home_dir = Some(tmp.path().join("home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let app = router(state.clone(), None);

    // Built in, not installed, not signed in.
    let (s, v) = call(&app, "GET", "/api/host/settings", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let a = antigravity(&v);
    assert_eq!((a["label"].as_str(), a["installer"].as_str()), (Some("Antigravity"), Some("antigravity-acp")), "{a}");
    assert_eq!(
        (a["found"].clone(), a["installedVersion"].clone(), a["signedIn"].clone()),
        (json!(null), json!(null), json!(null))
    );
    // Offline (as `AppState::new` is): nothing to install from.
    let (s, v) = call(&app, "POST", "/api/host/acp-agents/antigravity/install", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("registry_unavailable")), "{v}");
    // Only profiles PDT installs.
    let (s, _) = call(&app, "POST", "/api/host/acp-agents/cursor/install", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let auth = tmp.path().join("signed-in");
    #[cfg(unix)]
    let program = format!("#!/bin/sh\nFAKE_ACP_AUTH='{}' exec '{FAKE}' --plain acp\n", auth.display());
    #[cfg(not(unix))]
    let program = "not run on Windows".to_string();
    let zip = zip_of("agy_acp_server.par", program.as_bytes());
    let sha = hex::encode(sha2::Sha256::digest(&zip));

    // A download that doesn't match the registry's sha256 is not installed.
    let mut st = state.clone();
    st.toolchains = source(&zip, &"0".repeat(64));
    let (s, v) = call(&router(st, None), "POST", "/api/host/acp-agents/antigravity/install", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("checksum_mismatch")), "{v}");
    assert!(!tmp.path().join("data").join("toolchains").join("antigravity-acp").join("1.4.5").exists());

    let mut st = state.clone();
    st.toolchains = source(&zip, &sha);
    let app = router(st, None);
    let (s, v) = call(&app, "POST", "/api/host/acp-agents/antigravity/install", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let a = antigravity(&v);
    assert_eq!(a["installedVersion"], "1.4.5", "{a}");
    let installed = tmp.path().join("data").join("toolchains").join("antigravity-acp").join("1.4.5");
    assert!(installed.join("agy_acp_server.par").is_file());
    #[cfg(unix)]
    assert_eq!(
        std::path::Path::new(a["found"].as_str().unwrap()).canonicalize().unwrap(),
        installed.join("agy_acp_server.par").canonicalize().unwrap()
    );
    // Again: already the newest.
    let (s, v) = call(&app, "POST", "/api/host/acp-agents/antigravity/install", None).await;
    assert_eq!((s, antigravity(&v)["installedVersion"].as_str()), (StatusCode::OK, Some("1.4.5")));

    // A path set over it wins; blank goes back to the installed one.
    let (s, v) =
        call(&app, "PUT", "/api/host/settings", Some(json!({"acpPrograms": {"antigravity": "~/agy/server"}}))).await;
    assert_eq!(
        (s, antigravity(&v)["program"].as_str(), antigravity(&v)["found"].clone()),
        (StatusCode::OK, Some("~/agy/server"), json!(null)),
        "{v}"
    );
    let (s, v) = call(&app, "PUT", "/api/host/settings", Some(json!({"acpPrograms": {"antigravity": ""}}))).await;
    assert_eq!((s, antigravity(&v)["program"].clone()), (StatusCode::OK, json!(null)));

    #[cfg(unix)]
    {
        let (s, p) =
            call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
        assert_eq!(s, StatusCode::CREATED, "{p}");
        let body = json!({"projectId": p["id"], "kind": "agent_tui", "agent": "acp:antigravity", "cols": 80,
                          "rows": 24, "initialPrompt": "who"});
        let wait = |id: String, what: &'static str, f: fn(&[Value]) -> bool| {
            let app = app.clone();
            async move {
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    let all = events(&app, &id).await;
                    if f(&all) {
                        return all;
                    }
                    assert!(Instant::now() < deadline, "never saw {what}: {all:#?}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        };
        // Not signed in: starting it says where to sign in.
        let (s, v) = call(&app, "POST", "/api/sessions", Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_GATEWAY, "{v}");
        assert!(v["message"].as_str().unwrap().contains("Antigravity is not signed in"), "{v}");

        // Sign in: the server is asked for its Google sign-in.
        let (s, v) = call(&app, "POST", "/api/host/acp-agents/antigravity/sign-in", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), "oauth-personal");

        let (s, v) = call(&app, "POST", "/api/sessions", Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
        let all = wait(v["id"].as_str().unwrap().into(), "the first turn", |a| {
            a.iter().any(|e| e["event"]["kind"] == "turn" && e["event"]["phase"] == "ended")
        })
        .await;
        let said: Vec<&str> = all
            .iter()
            .filter(|e| e["event"]["kind"] == "agent_message")
            .filter_map(|e| e["event"]["markdown"].as_str())
            .collect();
        // Ask runs in Antigravity's `default` mode.
        assert!(said.iter().any(|m| m.contains("mode=default")), "{said:?}");
    }
}
