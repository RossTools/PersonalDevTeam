//! Workflow helpers shared by the tests of workflows' triggers and actions (pdt-api's `integrations` tests and the
//! integration crates' own, which include this folder by path): a PDT with given integrations enabled, running a
//! workflow now, connections, and fake HTTP services.

use super::call;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pdt_api::{AppState, router};
use pdt_forge::testing::FakeForge;
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct Env {
    pub app: axum::Router,
    pub state: AppState,
    pub gh: Arc<FakeForge>,
    pub tmp: tempfile::TempDir,
}

/// A PDT with these integrations built in and enabled, and `FakeForge` for GitHub.
pub async fn setup(integrations: Vec<Arc<dyn pdt_api::Integration>>) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    let ids: Vec<String> = integrations.iter().map(|i| i.id().to_string()).collect();
    state.integrations = Arc::new(pdt_api::Integrations::new(integrations));
    state.integrations.enable(&ids);
    let gh = Arc::new(FakeForge::default());
    state.forges = pdt_forge::ForgeRegistry::new(vec![std::sync::Arc::new(pdt_forge::HostedForge::github(gh.clone()))]);
    let app = router(state.clone(), None);
    Env { app, state, gh, tmp }
}

pub async fn create(env: &Env, doc: Value) -> String {
    let (s, w) = call(&env.app, "POST", "/api/workflows", Some(json!({"doc": doc, "enabled": true}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w["id"].as_str().unwrap().to_string()
}

pub async fn run_now(env: &Env, wf: &str, input: Value) -> Value {
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    let id = r["runId"].as_str().unwrap();
    call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1
}

pub async fn runs(env: &Env, wf: &str) -> Vec<Value> {
    let (_, r) = call(&env.app, "GET", &format!("/api/workflow-runs?workflow={wf}"), None).await;
    r.as_array().unwrap().clone()
}

pub async fn connection(env: &Env, name: &str, kind: &str, url: &str, config: Value, secret: Option<&str>) {
    let (s, c) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": name, "kind": kind, "url": url, "config": config})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    if let Some(secret) = secret {
        let id = c["id"].as_str().unwrap();
        let (s, _) =
            call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": secret}))).await;
        assert_eq!(s, StatusCode::OK);
    }
}

pub fn step<'a>(run: &'a Value, node: &str) -> &'a Value {
    run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|s| s["nodeId"] == node)
        .unwrap_or_else(|| panic!("{node} in {run}"))
}

pub fn manual_then(node: Value) -> Value {
    json!({"name": "Test", "nodes": [{"id": "go", "kind": "manual", "version": 1}, node],
        "edges": [{"from": "go", "to": node["id"]}]})
}

// ---- a fake HTTP service ----

/// One request a fake server got: method, path and query, JSON body, its key (Authorization, else X-Api-Key, else
/// a token header).
pub type Request1 = (String, String, Value, Option<String>);

#[derive(Clone, Default)]
pub struct Seen(pub Arc<Mutex<Vec<Request1>>>);

impl Seen {
    pub fn all(&self) -> Vec<Request1> {
        self.0.lock().unwrap().clone()
    }
}

/// A server on a port the OS picks: records each request (method, path and query, JSON body, key) and
/// answers what `answer` says for its path.
pub async fn fake_server(answer: fn(&str, usize) -> (u16, Value)) -> (String, Seen) {
    let seen = Seen::default();
    let s2 = seen.clone();
    let app = axum::Router::new().fallback(move |req: Request<Body>| {
        let seen = s2.clone();
        async move {
            let method = req.method().to_string();
            let path = req.uri().to_string();
            let auth = ["authorization", "x-api-key"]
                .iter()
                .find_map(|h| req.headers().get(*h))
                .or_else(|| req.headers().iter().find(|(k, _)| k.as_str().ends_with("-token")).map(|(_, v)| v))
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let bytes = req.into_body().collect().await.unwrap().to_bytes();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let n = {
                let mut all = seen.0.lock().unwrap();
                all.push((method, path.clone(), body, auth));
                all.len()
            };
            let (status, v) = answer(&path, n);
            (StatusCode::from_u16(status).unwrap(), axum::Json(v))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

/// A server that never answers request number `hang` (nor records it: as if PDT stopped before sending it), and
/// answers every other with `{"id": "m<n>"}`.
pub async fn hanging_server(hang: usize) -> (String, Seen) {
    let seen = Seen::default();
    let s2 = seen.clone();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = axum::Router::new().fallback(move |req: Request<Body>| {
        let (seen, count) = (s2.clone(), count.clone());
        async move {
            let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if n == hang {
                std::future::pending::<()>().await;
            }
            let method = req.method().to_string();
            let path = req.uri().to_string();
            let bytes = req.into_body().collect().await.unwrap().to_bytes();
            let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            seen.0.lock().unwrap().push((method, path, body, None));
            axum::Json(json!({"id": format!("m{n}")}))
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

/// Starts a run of `wf` and runs the engine once, with ticks that wait at most a moment for a slow step.
pub async fn start_slow(env: &mut Env, wf: &str, input: Value) -> String {
    env.state.engine_tick_wait = std::time::Duration::from_millis(300);
    let (s, r) = call(&env.app, "POST", &format!("/api/workflows/{wf}/run"), Some(json!({"input": input}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    pdt_api::run_workflows(&env.state).await;
    r["runId"].as_str().unwrap().to_string()
}

/// A second daemon on the same database and keychain: the first one stopped.
pub async fn restarted(env: &Env) -> (AppState, axum::Router) {
    let store = Store::open(&env.tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, env.tmp.path().join("data"));
    state.secrets = env.state.secrets.clone();
    state.integrations = env.state.integrations.clone();
    let app = router(state.clone(), None);
    (state, app)
}

/// A local address nothing listens on (the OS picked it, then it was let go).
pub async fn closed_address() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    format!("http://{}", listener.local_addr().unwrap())
}
