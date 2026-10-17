//! Demo data for dev instances (`PDT_SEED=demo`): two small git repositories inside the data folder, registered as
//! projects, with a worktree, a few ended sessions and a few past runs, so every list in the UI has something in it.
//! Only an empty database is seeded, and nothing is started: the sessions are finished and the runs are over.

use anyhow::Context;
use pdt_core::project::Project;
use pdt_core::run::{Run, RunKind, RunState};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use pdt_core::worktree::{WORKTREES_DIR, Worktree, worktree_path};
use pdt_store::Store;
use std::path::{Path, PathBuf};

/// Where the demo repositories live, inside the data folder.
pub const DEMO_DIR: &str = "demo";

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

const APP_CONFIG: &str = r#"# Demo run configuration. "web" serves this folder on the run's port; "check" is a test that always passes.
[[run]]
name    = "web"
command = "python3 -m http.server $PORT"
web     = true

[[test]]
name    = "check"
command = "git status --short"
"#;

const LIB_CONFIG: &str = r#"[[test]]
name    = "check"
command = "git status --short"
"#;

/// Seeds the demo data unless the database already has a project. Returns whether it seeded. `now` is in ms.
pub async fn seed_demo(store: &Store, data_dir: &Path, now: i64) -> anyhow::Result<bool> {
    if !store.list_projects().await?.is_empty() {
        return Ok(false);
    }
    let root = data_dir.join(DEMO_DIR);
    let repos = tokio::task::spawn_blocking(move || make_repos(&root)).await??;
    let id = || uuid::Uuid::now_v7().to_string();
    let path = |p: &Path| p.to_string_lossy().into_owned();

    let app = Project {
        id: id(),
        name: "Sample App".into(),
        repo_path: path(&repos.app),
        base_branch: "main".into(),
        created_at: now - 3 * DAY,
        key: String::new(),
        github_repo: None,
    };
    let lib = Project {
        id: id(),
        name: "Sample Library".into(),
        repo_path: path(&repos.lib),
        base_branch: "main".into(),
        created_at: now - 2 * DAY,
        key: String::new(),
        github_repo: None,
    };
    let wt = Worktree {
        id: id(),
        project_id: app.id.clone(),
        branch: repos.feature_branch.clone(),
        path: path(&repos.feature_path),
        base_commit: repos.feature_base.clone(),
        created_at: now - 5 * HOUR,
        missing: false,
    };
    for p in [&app, &lib] {
        store.insert_project(p).await?;
    }
    store.insert_worktree(&wt).await?;

    let session = |project: &Project, wt: Option<&Worktree>, agent: Option<AgentKind>, title: &str, ago: i64| Session {
        id: id(),
        project_id: project.id.clone(),
        worktree_id: wt.map(|w| w.id.clone()),
        kind: if agent.is_some() { SessionKind::AgentTui } else { SessionKind::Shell },
        agent,
        bypass_permissions: false,
        title: title.into(),
        cwd: wt.map_or_else(|| project.repo_path.clone(), |w| w.path.clone()),
        status: SessionStatus::Finished,
        exit_code: Some(0),
        created_at: now - ago,
        ended_at: Some(now - ago + 25 * MINUTE),
        external_session_id: None,
        fresh_started_at: None,
        activity: None,
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
    };
    let sessions = [
        session(&app, None, None, "Shell · main", 3 * DAY - HOUR),
        Session {
            exit_code: Some(1),
            ..session(&app, None, Some(AgentKind::Codex), "Codex · Fix the README typo", DAY)
        },
        session(&app, Some(&wt), Some(AgentKind::Claude), "Claude · Build the login form", 4 * HOUR),
        session(&app, Some(&wt), None, "Shell · feature/login-form", 3 * HOUR),
        Session {
            bypass_permissions: true,
            ..session(&lib, None, Some(AgentKind::Claude), "Claude · Write the docs (no prompts)", 2 * HOUR)
        },
    ];
    for s in &sessions {
        store.insert_session(s).await?;
        // Each session is a tab in its project's workspace, as sessions made through the API are.
        if let Some(home) = store.project_workspace_id(&s.project_id).await? {
            store
                .open_tab(&id(), &home, &pdt_core::workspace::TabItem::Session(s.id.clone()), None, s.created_at)
                .await?;
        }
    }

    let run = |project: &Project, wt: Option<&Worktree>, branch: &str, name: &str, kind: RunKind, ago: i64| Run {
        id: id(),
        project_id: project.id.clone(),
        worktree_id: wt.map(|w| w.id.clone()),
        branch: branch.into(),
        name: name.into(),
        kind,
        state: RunState::Passed,
        port: None,
        url: None,
        data_dir: None,
        cwd: wt.map_or_else(|| project.repo_path.clone(), |w| w.path.clone()),
        exit_code: Some(0),
        error: None,
        created_at: now - ago,
        updated_at: now - ago + 2 * MINUTE,
    };
    let runs = [
        run(&app, None, "main", "check", RunKind::Test, DAY + HOUR),
        Run {
            state: RunState::Stopped,
            exit_code: None,
            ..run(&app, Some(&wt), &repos.feature_branch, "web", RunKind::Run, 3 * HOUR)
        },
        Run {
            state: RunState::Failed,
            exit_code: Some(1),
            error: Some("tests failed (exit code 1)".into()),
            ..run(&app, Some(&wt), &repos.feature_branch, "check", RunKind::Test, 2 * HOUR)
        },
        run(&lib, None, "main", "check", RunKind::Test, HOUR),
    ];
    for r in &runs {
        store.insert_run(r).await?;
    }
    Ok(true)
}

struct DemoRepos {
    app: PathBuf,
    lib: PathBuf,
    feature_branch: String,
    feature_path: PathBuf,
    feature_base: String,
}

/// Builds the demo repositories under `root`, replacing whatever an earlier, interrupted seed left there.
fn make_repos(root: &Path) -> anyhow::Result<DemoRepos> {
    if root.exists() {
        std::fs::remove_dir_all(root).with_context(|| format!("clearing {}", root.display()))?;
    }
    let write = |dir: &Path, file: &str, text: &str| -> anyhow::Result<()> {
        let path = dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    };

    let app = root.join("sample-app");
    pdt_git::init(&app, "main")?;
    write(&app, "README.md", "# Sample App\n\nA demo project for PDT's dev instances. Teh page says hello.\n")?;
    write(&app, ".pdt/project.toml", APP_CONFIG)?;
    let first = pdt_git::commit_all(&app, "Initial commit")?;
    write(&app, "index.html", "<!doctype html>\n<title>Sample App</title>\n<h1>Hello from Sample App</h1>\n")?;
    pdt_git::commit_all(&app, "Add the greeting page")?;
    write(&app, "style.css", "body { font-family: system-ui; margin: 2rem; }\n")?;
    let base = pdt_git::commit_all(&app, "Style the page")?;
    pdt_git::create_branch(&app, "fix/readme-typo", &first)?;

    let feature_branch = "feature/login-form".to_string();
    let feature_path = worktree_path(&app, &feature_branch);
    pdt_git::ensure_excluded(&app, &format!("/{WORKTREES_DIR}/"))?;
    let feature_base = pdt_git::add_worktree(&app, &feature_path, &feature_branch, &base)?;
    write(&feature_path, "login.html", "<!doctype html>\n<form><input name=user><button>Log in</button></form>\n")?;
    pdt_git::commit_all(&feature_path, "Add a login form")?;
    // Uncommitted work in progress, as an agent would leave it.
    write(&feature_path, "login.css", "form { display: grid; gap: .5rem; }\n")?;

    let lib = root.join("sample-library");
    pdt_git::init(&lib, "main")?;
    write(&lib, "README.md", "# Sample Library\n\nA second demo project.\n")?;
    write(&lib, ".pdt/project.toml", LIB_CONFIG)?;
    pdt_git::commit_all(&lib, "Initial commit")?;

    // The paths git reports (resolved, e.g. /private/var on macOS), as adding the project in the UI would store them.
    let app = pdt_git::resolve_repo(&app)?.root;
    let lib = pdt_git::resolve_repo(&lib)?.root;
    let feature_path = worktree_path(&app, &feature_branch);
    Ok(DemoRepos { app, lib, feature_branch, feature_path, feature_base })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store(dir: &Path) -> Store {
        Store::open(&dir.join("pdt.db")).await.unwrap()
    }

    #[tokio::test]
    async fn seeds_an_empty_database_once() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path()).await;
        assert!(seed_demo(&store, tmp.path(), 10 * DAY).await.unwrap());

        let projects = store.list_projects().await.unwrap();
        assert_eq!(projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Sample App", "Sample Library"]);
        let app = &projects[0];
        let info = pdt_git::resolve_repo(Path::new(&app.repo_path)).unwrap();
        assert_eq!(info.current_branch.as_deref(), Some("main"));
        assert!(pdt_git::branch_exists(Path::new(&app.repo_path), "fix/readme-typo"));
        assert!(pdt_git::branch_file(Path::new(&app.repo_path), "main", ".pdt/project.toml").unwrap().is_some());

        let wts = store.list_worktrees(&app.id).await.unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].branch, "feature/login-form");
        assert!(Path::new(&wts[0].path).join("login.html").is_file());

        let sessions = store.list_sessions(&app.id).await.unwrap();
        assert_eq!(sessions.len(), 4);
        assert!(sessions.iter().all(|s| s.status == SessionStatus::Finished && s.ended_at.is_some()));
        assert!(sessions.iter().any(|s| s.worktree_id.as_deref() == Some(wts[0].id.as_str())));
        assert!(store.list_sessions_by_status(SessionStatus::Interrupted).await.unwrap().is_empty());
        let workspaces = store.list_workspaces().await.unwrap();
        let tabs = |name: &str| workspaces.iter().find(|w| w.name == name).map(|w| w.tabs.len());
        assert_eq!((tabs("Sample App"), tabs("Sample Library")), (Some(4), Some(1)), "every session has a tab");

        let runs = store.list_runs(&app.id).await.unwrap();
        assert_eq!(runs.len(), 3);
        assert!(store.list_active_runs().await.unwrap().is_empty());

        // A database with projects is left alone.
        assert!(!seed_demo(&store, tmp.path(), 11 * DAY).await.unwrap());
        assert_eq!(store.list_projects().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn replaces_repos_left_by_an_interrupted_seed() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(DEMO_DIR).join("sample-app")).unwrap();
        std::fs::write(tmp.path().join(DEMO_DIR).join("sample-app").join("stray"), "x").unwrap();
        let store = store(tmp.path()).await;
        assert!(seed_demo(&store, tmp.path(), 10 * DAY).await.unwrap());
        assert!(!tmp.path().join(DEMO_DIR).join("sample-app").join("stray").exists());
    }
}
