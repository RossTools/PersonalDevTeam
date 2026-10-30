//! GitLab (PDT-246) through the API: a project on a self-hosted GitLab gets the GitLab forge once the user signs in to
//! that host (a pasted token), and a started ticket runs end to end on it: its merge request and pipeline are read by
//! the PR poller, and Merge sets the merge request to merge when the pipeline succeeds. GitLab is a fake transport
//! answering like GitLab's API (never the network); the agent is a stand-in.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use axum::http::StatusCode;
use common::{argv_agent, call, git};
use pdt_api::{AppState, router};
use pdt_forge::transport::{Request, Response, Transport};
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const API: &str = "https://gitlab.example.com/api/v4/";
const PROJECT: &str = "projects/group%2Fsub%2Fapp";

/// A GitLab server's answers for `group/sub/app`.
#[derive(Default)]
struct FakeGitlab {
    /// Merge requests, newest first.
    mrs: Vec<Value>,
    pipelines: Vec<Value>,
    /// Method and path (with query) of each request, and its body.
    seen: Vec<(String, String, Option<String>)>,
    /// The bearer tokens sent.
    tokens: Vec<String>,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<FakeGitlab>>);

impl Transport for Fake {
    fn send(&self, req: &Request) -> Result<Response, String> {
        let mut g = self.0.lock().unwrap();
        let path = req.url.strip_prefix(API).ok_or_else(|| format!("not the fake GitLab: {}", req.url))?.to_string();
        g.seen.push((req.method.to_string(), path.clone(), req.body.clone()));
        if let Some((_, t)) = req.headers.iter().find(|(k, _)| *k == "Authorization") {
            g.tokens.push(t.clone());
        }
        let (route, query) = path.split_once('?').unwrap_or((&path, ""));
        let param = |k: &str| {
            query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).unwrap_or_default().replace("%2F", "/")
        };
        let ok = |v: Value| Ok(Response::new(200, v.to_string()));
        let rest = route.strip_prefix(PROJECT);
        match (req.method, route, rest) {
            ("GET", "user", _) => ok(json!({"id": 7, "username": "rwest"})),
            ("GET", "personal_access_tokens/self", _) => ok(json!({"scopes": ["api"]})),
            ("GET", _, Some("")) => ok(json!({"merge_method": "merge", "squash_option": "default_off",
                                             "merge_pipelines_enabled": false, "merge_trains_enabled": false})),
            ("GET", _, Some("/merge_requests")) => {
                let branch = param("source_branch");
                ok(json!(g.mrs.iter().filter(|m| m["source_branch"] == branch.as_str()).cloned().collect::<Vec<_>>()))
            }
            ("GET", _, Some("/pipelines")) => {
                let sha = param("sha");
                ok(json!(g.pipelines.iter().filter(|p| p["sha"] == sha.as_str()).cloned().collect::<Vec<_>>()))
            }
            ("PUT", _, Some("/merge_requests/1/merge")) => {
                if let Some(m) = g.mrs.first_mut() {
                    m["state"] = json!("merged");
                }
                ok(g.mrs.first().cloned().unwrap_or_default())
            }
            _ => Ok(Response::new(404, r#"{"message":"404 Not found"}"#)),
        }
    }
}

struct Env {
    app: axum::Router,
    state: AppState,
    gitlab: Fake,
    project: String,
    repo: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join(".gitlab-ci.yml"), "test:\n  script: [make test]\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["remote", "add", "origin", "git@gitlab.example.com:group/sub/app.git"]);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.claude_program = argv_agent(&bin, "fakeclaude").to_string_lossy().into_owned();
    state.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(pdt_int_gitlab::Gitlab)]));
    state.integrations.enable(&["gitlab".into()]);
    let gitlab = Fake::default();
    let api = Arc::new(pdt_int_gitlab::GitlabApi::new(Arc::new(gitlab.clone())));
    state.forges = pdt_forge::ForgeRegistry::new(vec![Arc::new(pdt_int_gitlab::GitlabPlugin::new(api))]);
    let app = router(state.clone(), None);
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "App", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, gitlab, project: p["id"].as_str().unwrap().into(), repo, _tmp: tmp }
}

fn head(repo: &std::path::Path, rev: &str) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(repo).args(["rev-parse", rev]).output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[tokio::test]
async fn a_self_hosted_project_gets_the_gitlab_forge_once_signed_in_and_runs_a_ticket_end_to_end() {
    let env = setup().await;
    let forge = format!("/api/projects/{}/forge", env.project);
    let (_, f) = call(&env.app, "GET", &forge, None).await;
    assert!(f.is_null(), "no login for gitlab.example.com yet: {f}");

    // The login: Test says who the token is, Save keeps it as that host's account.
    let (_, view) = call(&env.app, "GET", "/api/forges/gitlab/login", None).await;
    assert_eq!(view["defaultApiUrl"], "https://gitlab.com/api/v4");
    assert_eq!(
        (view["hasDeviceFlow"].clone(), view["deviceFlow"].clone()),
        (json!(true), json!(false)),
        "no app on gitlab.com"
    );
    let body = json!({"token": "glpat-x", "apiUrl": "https://gitlab.example.com"});
    let (s, t) = call(&env.app, "POST", "/api/forges/gitlab/login/test", Some(body.clone())).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!((t["login"].as_str(), t["host"].as_str()), (Some("rwest"), Some("gitlab.example.com")));
    assert_eq!(t["scopes"], json!(["api"]));
    let (s, v) = call(&env.app, "POST", "/api/forges/gitlab/login/token", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["accounts"][0]["apiUrl"], "https://gitlab.example.com/api/v4");
    let stored = env.state.secrets.get(&env.state.forge_logins_account).unwrap().unwrap();
    assert!(stored.contains("gitlab:gitlab.example.com"), "{stored}");

    let (_, f) = call(&env.app, "GET", &forge, None).await;
    assert_eq!(
        (f["kind"].as_str(), f["repo"].as_str(), f["name"].as_str()),
        (Some("gitlab"), Some("group/sub/app"), Some("GitLab"))
    );
    assert_eq!(f["queueName"], "merge train");

    // A ticket started on it: its branch's merge request and the head's pipeline are its PR and CI.
    let (_, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": "Ship it"})))
            .await;
    let ticket = t["id"].as_str().unwrap().to_string();
    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{ticket}/start"),
        Some(json!({"agent": "claude", "sessionMode": "terminal"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
    let attempt = started["attempt"]["id"].as_str().unwrap().to_string();
    let sha = head(&env.repo, "main");
    {
        let mut g = env.gitlab.0.lock().unwrap();
        g.mrs = vec![json!({
            "iid": 1, "project_id": 9, "source_project_id": 9, "title": "Ship it", "state": "opened", "draft": false,
            "sha": sha, "source_branch": branch, "target_branch": "main", "detailed_merge_status": "mergeable",
            "has_conflicts": false, "merge_when_pipeline_succeeds": false, "created_at": "2099-01-01T00:00:00.000Z",
            "web_url": "https://gitlab.example.com/group/sub/app/-/merge_requests/1"
        })];
        g.pipelines = vec![json!({"id": 501, "sha": sha, "ref": branch, "status": "success",
            "created_at": "2099-01-01T00:01:00.000Z", "web_url": "https://gitlab.example.com/group/sub/app/-/pipelines/501"})];
    }
    pdt_api::refresh_prs(&env.state).await;
    let (_, t) = call(&env.app, "GET", &format!("/api/tickets/{ticket}"), None).await;
    let pr = &t["attempt"]["pr"];
    assert_eq!((pr["number"].clone(), pr["state"].clone()), (json!(1), json!("open")), "{t}");
    assert_eq!(pr["url"], "https://gitlab.example.com/group/sub/app/-/merge_requests/1");
    assert_eq!(pr["ci"]["state"], "passed", "{pr}");
    assert_eq!(pr["ci"]["runs"][0]["provider"], "gitlab_ci");

    // Merge: set to merge when the pipeline succeeds, as the user's login.
    let (s, m) = call(&env.app, "POST", &format!("/api/attempts/{attempt}/merge"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    {
        let g = env.gitlab.0.lock().unwrap();
        let put = g.seen.iter().find(|(m, p, _)| m == "PUT" && p.ends_with("/merge_requests/1/merge")).expect("merged");
        let body: Value = serde_json::from_str(put.2.as_deref().unwrap()).unwrap();
        assert_eq!(
            (body["auto_merge"].clone(), body["merge_when_pipeline_succeeds"].clone()),
            (json!(true), json!(true))
        );
        assert!(g.tokens.iter().all(|t| t == "Bearer glpat-x"), "{:?}", g.tokens);
    }

    // GitLab can't be turned off while it is the project's forge.
    let (s, e) = call(&env.app, "PUT", "/api/integrations", Some(json!({"enabled": []}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    assert_eq!(e["code"], "in_use");
    let (_, list) = call(&env.app, "GET", "/api/integrations", None).await;
    let gl = list.as_array().unwrap().iter().find(|p| p["id"] == "gitlab").unwrap();
    assert_eq!((gl["forgeKind"].as_str(), gl["forgeProjects"][0]["name"].as_str()), (Some("gitlab"), Some("App")));
}
