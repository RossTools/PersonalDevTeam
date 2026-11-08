//! Workflow triggers on releases, disk space, failing connections and todos (PDT-365): each fires once per
//! occurrence with its payload, and a level that stays where it is fires nothing. The forge is `FakeForge`, the disk a
//! probe the test sets. Updates are in `update.rs`, Scryer in its crate.

use crate::common;

use axum::http::StatusCode;
use common::workflows::{Env, create, runs, setup};
use common::{call, git};
use pdt_core::storage::DiskSpace;
use pdt_core::workflow::api::LogEvent;
use pdt_forge::{Release, ReleaseAsset};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const REPO: &str = "example-org/thing";
const GB: u64 = 1024 * 1024 * 1024;

async fn events(env: &Env, kind: &str) -> Vec<LogEvent> {
    let f = pdt_store::EventFilter { kind: Some(kind.into()), limit: 100, ..Default::default() };
    env.state.store.list_log_events(&f).await.unwrap()
}

/// A workflow started by `trigger` that ends at once.
fn on(name: &str, trigger: &str) -> Value {
    json!({"name": name, "nodes": [
        {"id": "t", "kind": trigger, "version": 1, "config": {}},
        {"id": "e", "kind": "end", "version": 1}],
        "edges": [{"from": "t", "to": "e"}]})
}

/// A project whose origin is `REPO` on GitHub; its id.
async fn project(env: &Env) -> String {
    let repo = env.tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    git(&repo, &["remote", "add", "origin", &format!("https://github.com/{REPO}.git")]);
    let body = json!({"name": "Thing", "key": "TH", "repoPath": repo.to_string_lossy()});
    let (s, p) = call(&env.app, "POST", "/api/projects", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    p["id"].as_str().unwrap().to_string()
}

fn release(tag: &str, prerelease: bool) -> Release {
    Release {
        tag: tag.into(),
        name: format!("Thing {tag}"),
        url: format!("https://github.com/{REPO}/releases/tag/{tag}"),
        published_at: Some("2026-11-01T12:00:00Z".into()),
        notes: "### Added\n- things".into(),
        prerelease,
        assets: vec![ReleaseAsset {
            name: "thing.tar.gz".into(),
            url: "https://dl/thing.tar.gz".into(),
            size: Some(9),
        }],
    }
}

fn set_releases(env: &Env, list: Vec<Release>) {
    env.gh.releases.lock().unwrap().insert(REPO.into(), list);
}

#[tokio::test]
async fn a_new_release_fires_once_and_the_first_look_only_records() {
    let env = setup(vec![]).await;
    let project = project(&env).await;
    set_releases(&env, vec![release("v0.1.0", false)]);

    // Nothing watches releases: the forge isn't asked.
    pdt_api::refresh_releases(&env.state).await;
    assert!(!env.gh.calls.lock().unwrap().iter().any(|c| c.starts_with("recent releases")));

    let wf = create(&env, on("after a release", "release.published")).await;
    pdt_api::refresh_releases(&env.state).await;
    assert!(events(&env, "release.published").await.is_empty(), "the first look only records");

    set_releases(&env, vec![release("v0.2.0-rc.1", true), release("v0.1.0", false)]);
    pdt_api::refresh_releases(&env.state).await;
    pdt_api::refresh_releases(&env.state).await;
    let got = events(&env, "release.published").await;
    assert_eq!(got.len(), 1, "{got:?}");
    let p = &got[0].payload;
    assert_eq!(
        (&p["projectId"], &p["forge"], &p["repo"], &p["tag"], &p["name"], &p["prerelease"]),
        (
            &json!(project),
            &json!("github"),
            &json!(REPO),
            &json!("v0.2.0-rc.1"),
            &json!("Thing v0.2.0-rc.1"),
            &json!(true)
        )
    );
    assert_eq!(p["assets"], json!([{"name": "thing.tar.gz", "url": "https://dl/thing.tar.gz", "size": 9}]));
    assert_eq!(got[0].project_id.as_deref(), Some(project.as_str()));

    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0]["trigger"]["tag"], "v0.2.0-rc.1");
}

/// A disk probe the test moves.
fn disk(env: &mut Env) -> Arc<AtomicU64> {
    let free = Arc::new(AtomicU64::new(500 * GB));
    let seen = free.clone();
    env.state.disk_space = Arc::new(move |_| Some(DiskSpace { free: seen.load(Ordering::SeqCst), total: 1000 * GB }));
    free
}

#[tokio::test]
async fn disk_levels_fire_once_per_change_not_every_check() {
    let mut env = setup(vec![]).await;
    let free = disk(&mut env);
    let check = |gb: u64| {
        free.store(gb * GB, Ordering::SeqCst);
        pdt_api::storage::watch_disks(&env.state)
    };
    check(500).await;
    assert!(events(&env, "disk.low").await.is_empty() && events(&env, "disk.recovered").await.is_empty());

    // Low (the default thresholds: 20 GB low, 5 GB critical), twice: once.
    check(12).await;
    check(11).await;
    let low = events(&env, "disk.low").await;
    assert_eq!(low.len(), 1, "{low:?}");
    let p = &low[0].payload;
    assert_eq!((&p["level"], &p["from"], &p["free"]), (&json!("low"), &json!("ok"), &json!(12 * GB)));
    assert_eq!(
        (&p["total"], &p["lowGb"], &p["criticalGb"], &p["reclaimable"]),
        (&json!(1000 * GB), &json!(20), &json!(5), &json!(0))
    );
    assert!(!p["folders"].as_array().unwrap().is_empty(), "{p}");

    // Critical is lower still: news. Back to low is not.
    check(3).await;
    check(2).await;
    check(12).await;
    let low = events(&env, "disk.low").await;
    assert_eq!(low.len(), 2, "{low:?}");
    assert!(low.iter().any(|e| e.payload["level"] == "critical" && e.payload["from"] == "low"));

    check(400).await;
    check(400).await;
    let back = events(&env, "disk.recovered").await;
    assert_eq!(back.len(), 1, "{back:?}");
    assert_eq!((&back[0].payload["level"], &back[0].payload["from"]), (&json!("ok"), &json!("low")));
}

#[tokio::test]
async fn a_connection_fires_once_each_time_it_starts_failing() {
    let env = setup(vec![]).await;
    let body = json!({"name": "svc", "kind": "http", "url": "http://127.0.0.1:9", "config": {}});
    let (s, c) = call(&env.app, "POST", "/api/connections", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let report = |e: Option<&'static str>| pdt_api::sdk::connections::report(&env.state, "svc", e);
    report(None).await;
    report(Some("401 from svc")).await;
    report(Some("401 from svc")).await;
    report(Some("still 401")).await;
    let got = events(&env, "connection.failed").await;
    assert_eq!(got.len(), 1, "{got:?}");
    let p = &got[0].payload;
    assert_eq!(
        (&p["name"], &p["kind"], &p["source"], &p["error"]),
        (&json!("svc"), &json!("http"), &json!("connection"), &json!("401 from svc"))
    );
    // It works again, then fails again: news again.
    report(None).await;
    report(Some("down")).await;
    assert_eq!(events(&env, "connection.failed").await.len(), 2);
}

/// A GitHub whose one account the forge has refused (since `rejected_at`).
struct Refused(pdt_forge::HostedForge);

impl pdt_forge::login::ForgeLogin for Refused {
    fn spec(&self) -> pdt_forge::login::LoginSpec {
        pdt_forge::login::LoginSpec {
            default_api_url: None,
            api_url_example: String::new(),
            scopes: String::new(),
            token_help: String::new(),
            device_flow: false,
        }
    }
    fn api_url(&self, given: &str) -> String {
        given.into()
    }
    fn host_of(&self, _api_url: &str) -> String {
        "github.com".into()
    }
    fn web_url(&self, _api_url: &str) -> String {
        "https://github.com".into()
    }
    fn whoami(&self, _api_url: &str, _token: &str) -> Result<pdt_forge::login::Whoami, String> {
        Err("refused".into())
    }
    fn set_account(&self, _host: &str, _account: Option<pdt_forge::login::ForgeAccount>) {}
    fn accounts(&self) -> Vec<pdt_forge::login::AccountState> {
        let account = pdt_forge::login::ForgeAccount { login: "octo".into(), ..Default::default() };
        vec![pdt_forge::login::AccountState { host: "github.com".into(), account, rejected_at: Some(7) }]
    }
}

impl pdt_forge::ForgePlugin for Refused {
    fn kind(&self) -> pdt_core::forge::ForgeKind {
        self.0.kind()
    }
    fn label(&self) -> &'static str {
        self.0.label()
    }
    fn claims_host(&self, host: &str) -> bool {
        self.0.claims_host(host)
    }
    fn forge(&self) -> Arc<dyn pdt_forge::Forge> {
        self.0.forge()
    }
    fn login(&self) -> Option<&dyn pdt_forge::login::ForgeLogin> {
        Some(self)
    }
}

#[tokio::test]
async fn a_refused_forge_login_fires_once() {
    let mut env = setup(vec![]).await;
    let plugin = Refused(pdt_forge::testing::fake_github(&env.gh));
    env.state.forges = pdt_forge::ForgeRegistry::new(vec![Arc::new(plugin)]);
    pdt_api::forge_login::check(&env.state).await;
    pdt_api::forge_login::check(&env.state).await;
    let got = events(&env, "connection.failed").await;
    assert_eq!(got.len(), 1, "{got:?}");
    let p = &got[0].payload;
    assert_eq!(
        (&p["name"], &p["kind"], &p["source"]),
        (&json!("GitHub (github.com)"), &json!("github"), &json!("forge"))
    );
    assert!(p["error"].as_str().unwrap().contains("octo"), "{p}");
    // Once the event log has forgotten it, the same refusal is still not news.
    pdt_api::run_workflows(&env.state).await;
    assert!(env.state.store.prune_events(i64::MAX).await.unwrap() > 0);
    pdt_api::forge_login::check(&env.state).await;
    assert!(events(&env, "connection.failed").await.is_empty());
}

#[tokio::test]
async fn a_todo_coming_due_fires_once() {
    let env = setup(vec![]).await;
    // A week either side of today, so a run across midnight sees the same: one overdue, one not yet due.
    let today = jiff::Zoned::now().date();
    let yesterday = today.checked_sub(jiff::Span::new().days(7)).unwrap().to_string();
    let tomorrow = today.checked_add(jiff::Span::new().days(7)).unwrap().to_string();
    let add = |body: Value| async {
        let (s, t) = call(&env.app, "POST", "/api/todos", Some(body)).await;
        assert_eq!(s, StatusCode::CREATED, "{t}");
        t["id"].as_str().unwrap().to_string()
    };
    let due = add(json!({"title": "Pay rent", "body": "the landlord's account", "dueOn": yesterday})).await;
    add(json!({"title": "Later", "dueOn": tomorrow})).await;
    add(json!({"title": "Undated"})).await;
    pdt_api::sync_notifications(&env.state).await;
    pdt_api::sync_notifications(&env.state).await;
    let got = events(&env, "todo.due").await;
    assert_eq!(got.len(), 1, "{got:?}");
    let p = &got[0].payload;
    assert_eq!(
        (&p["todoId"], &p["title"], &p["body"], &p["dueOn"], &p["dueTime"], &p["overdue"]),
        (
            &json!(due),
            &json!("Pay rent"),
            &json!("the landlord's account"),
            &json!(yesterday),
            &Value::Null,
            &json!(true)
        )
    );
    assert!(p["dueAt"].as_i64().unwrap() > 0);
}
