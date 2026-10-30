//! Demo data for dev instances (`PDT_SEED=demo`; the website's sample, `PDT_SEED=website`, is in `website.rs`): two small git repositories inside the data folder, registered as
//! projects, with a worktree, a few ended sessions and a few past runs, so every list in the UI has something in it.
//! Only an empty database is seeded, and nothing is started: the sessions are finished and the runs are over.

use anyhow::Context;
use pdt_core::project::Project;
use pdt_core::run::{Run, RunKind, RunState};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use pdt_core::ticket::{LinkKind, NewTicket, Priority, TicketStage};
use pdt_core::worktree::{WORKTREES_DIR, Worktree, worktree_path};
use pdt_store::Store;
use std::path::{Path, PathBuf};

pub mod website;

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

# Named parts of the app, for overlap between tickets. Only one active ticket at a time should change the styles.
[[area]]
name  = "pages"
paths = ["*.html"]

[[area]]
name      = "styles"
paths     = ["*.css"]
exclusive = true

[[area]]
name  = "docs"
paths = ["*.md", "docs/**"]

[[area]]
name  = "auth"
paths = ["auth/**"]
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
        forge: None,
        kind: Default::default(),
    };
    let lib = Project {
        id: id(),
        name: "Sample Library".into(),
        repo_path: path(&repos.lib),
        base_branch: "main".into(),
        created_at: now - 2 * DAY,
        key: String::new(),
        forge: None,
        kind: Default::default(),
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
        attempt_id: None,
        ticket: None,
        mode: pdt_core::conversation::SessionMode::Terminal,
        model: None,
        permission_mode: None,
        persona: None,
    };
    let sessions = [
        session(&app, None, None, "main", 3 * DAY - HOUR),
        Session { exit_code: Some(1), ..session(&app, None, Some(AgentKind::Codex), "Fix the README typo", DAY) },
        session(&app, Some(&wt), Some(AgentKind::Claude), "Build the login form", 4 * HOUR),
        session(&app, Some(&wt), None, "feature/login-form", 3 * HOUR),
        Session {
            bypass_permissions: true,
            ..session(&lib, None, Some(AgentKind::Claude), "Write the docs (no prompts)", 2 * HOUR)
        },
    ];
    // A Claude conversation with a finished turn (structured agents, Phase 5), so the conversation view has something
    // to show without running an agent.
    let conversation = Session {
        mode: pdt_core::conversation::SessionMode::Conversation,
        permission_mode: Some(pdt_core::conversation::PermissionMode::Ask),
        persona: None,
        exit_code: None,
        ..session(&app, Some(&wt), Some(AgentKind::Claude), "Validate the login form", 90 * MINUTE)
    };
    let sessions: Vec<Session> = sessions.into_iter().chain([conversation.clone()]).collect();
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
        detached: false,
        process: None,
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
    seed_conversation(store, &conversation.id, now - 90 * MINUTE).await?;
    seed_tickets(store, &app.id, &wt, now).await?;
    Ok(true)
}

/// One turn of a conversation: the user's request, commentary, three tool calls (one approved), the answer.
async fn seed_conversation(store: &Store, session_id: &str, at: i64) -> anyhow::Result<()> {
    use pdt_core::conversation::*;
    use serde_json::json;
    let turn = "seed-turn-1";
    let diff = "--- a/login.html\n+++ b/login.html\n@@ -2,4 +2,6 @@\n <form id=\"login\">\n-  <input name=\"email\">\n-  <input name=\"password\" type=\"password\">\n+  <input name=\"email\" type=\"email\" required>\n+  <span class=\"error\" data-for=\"email\"></span>\n+  <input name=\"password\" type=\"password\" required minlength=\"8\">\n+  <span class=\"error\" data-for=\"password\"></span>\n   <button>Sign in</button>\n </form>";
    let tool = |id: &str, tool: &str, title: &str, input: serde_json::Value, output: &str| AgentEvent::ToolCall {
        id: id.into(),
        tool: tool.into(),
        title: title.into(),
        input,
        status: ToolStatus::Done,
        output: Some(output.into()),
        exit_code: None,
        diff: None,
        parent_id: None,
    };
    let events: Vec<(Option<&str>, AgentEvent)> = vec![
        (Some("turn:seed-turn-1"), AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None }),
        (
            Some("u:seed-1"),
            AgentEvent::UserMessage {
                text: "Add inline validation to the login form: a required email and a password of at least 8 characters."
                    .into(),
                images: vec![],
                source: MessageSource::Pdt,
                pdt: None,
            },
        ),
        (
            Some("m:seed-1"),
            AgentEvent::AgentMessage {
                id: "seed-1".into(),
                markdown: "I'll look at the form first.".into(),
                phase: MessagePhase::Commentary,
            },
        ),
        (
            Some("t:seed-read"),
            tool("seed-read", "Read", "login.html", json!({"file_path": "login.html"}), "<form id=\"login\">…</form>"),
        ),
        (
            Some("r:seed-req"),
            AgentEvent::Request {
                id: "seed-req".into(),
                request_kind: RequestKind::Approval,
                tool: "Edit".into(),
                title: "login.html".into(),
                description: Some("Add required and minlength attributes".into()),
                input: json!({"file_path": "login.html"}),
                tool_call_id: Some("seed-edit".into()),
                can_always: true,
                diff: Some(diff.into()),
                state: RequestState::Answered,
                answer: Some(Answer::Approve { always: false }),
            },
        ),
        (
            Some("t:seed-edit"),
            {
                let mut edit =
                    tool("seed-edit", "Edit", "login.html", json!({"file_path": "login.html"}), "The file was updated.");
                if let AgentEvent::ToolCall { diff: d, .. } = &mut edit {
                    *d = Some(diff.into());
                }
                edit
            },
        ),
        (
            Some("t:seed-test"),
            tool("seed-test", "Bash", "npm test", json!({"command": "npm test"}), "> sample-app@1.0.0 test\n\n2 passing (12ms)"),
        ),
        (
            Some("m:seed-2"),
            AgentEvent::AgentMessage {
                id: "seed-2".into(),
                markdown: "Done. The login form now checks:\n\n- **Email**: required, and must look like an address\n- \
                           **Password**: at least 8 characters\n\nErrors show under each field as you type. The tests pass."
                    .into(),
                phase: MessagePhase::Final,
            },
        ),
        (
            Some("usage"),
            AgentEvent::Usage { context_used: Some(31_200), context_window: Some(200_000), rate_limits: vec![], cost_usd: Some(0.12) },
        ),
        (Some("diff:seed-turn-1"), AgentEvent::TurnDiff { diff: diff.into() }),
        (
            Some("turn-end:seed-turn-1"),
            AgentEvent::Turn { phase: TurnPhase::Ended, status: Some(TurnStatus::Completed), duration_ms: Some(48_000), reason: None },
        ),
    ];
    for (i, (item, event)) in events.iter().enumerate() {
        let ne =
            pdt_store::NewEvent { ts: at + i as i64 * 4_000, item_id: *item, turn_id: Some(turn), event, raw: None };
        store.upsert_session_event(session_id, ne).await?;
    }
    Ok(())
}

/// A small backlog for Sample App: every stage, a few priorities, labels and areas, one blocker, a comment, and an
/// active attempt on the login form's worktree (so the tickets sharing its areas show overlap).
async fn seed_tickets(store: &Store, project_id: &str, wt: &Worktree, now: i64) -> anyhow::Result<()> {
    use Priority::*;
    use TicketStage::*;
    type Item<'a> = (&'a str, &'a str, TicketStage, Priority, &'a [&'a str], &'a [&'a str]);
    let items: &[Item] = &[
        (
            "Build the login form",
            "Email and password, with inline errors.\n\n- [x] Layout\n- [ ] Validation\n- [ ] Submit",
            InProgress,
            High,
            &["ui"],
            &["pages", "styles"],
        ),
        ("Remember the signed-in user", "Keep the session across reloads.", Todo, Medium, &["auth"], &["auth"]),
        (
            "Password reset by email",
            "Send a one-time link; expire it after an hour.",
            Ready,
            Medium,
            &["auth"],
            &["auth", "pages"],
        ),
        ("Fix the README typo", "\"recieve\" in the install section.", Ready, Low, &["docs"], &["docs"]),
        ("Dark mode for the settings page", "", Draft, None, &["ui"], &["styles"]),
        (
            "Rate-limit the login endpoint",
            "Five tries a minute per address.",
            Draft,
            Urgent,
            &["auth", "security"],
            &[],
        ),
        ("Add a favicon", "", Draft, Low, &["ui"], &[]),
        ("Export data as CSV", "From the reports page.", Draft, None, &[], &[]),
        ("Set up the project", "Repository, CI and a first page.", Done, None, &[], &[]),
        ("Try a Vue port", "Decided against it.", WontDo, None, &[], &[]),
    ];
    let mut ids = Vec::new();
    // Each new ticket goes to the top of its stage, so create bottom-up to keep this list's order.
    for (title, description, stage, priority, labels, areas) in items.iter().rev() {
        let new = NewTicket {
            title: (*title).into(),
            description: Some((*description).into()),
            stage: Some(*stage),
            priority: Some(*priority),
            labels: Some(labels.iter().map(|l| (*l).to_string()).collect()),
            areas: Some(areas.iter().map(|a| (*a).to_string()).collect()),
            before: Option::None,
            after: Option::None,
            parent: Option::None,
            blocked_by_previous: Option::None,
        };
        let valid = new.validate().context("demo ticket")?;
        let t = store.create_ticket(&uuid::Uuid::now_v7().to_string(), project_id, &valid, now).await?;
        ids.push(t.summary.id);
    }
    ids.reverse();
    // "Remember the signed-in user" blocks "Password reset by email".
    store.add_link(&ids[1], &ids[2], LinkKind::Blocks, now).await?;
    store.add_comment(&ids[0], "Validation messages should match the design doc.", now).await?;
    // A ticket that takes several PRs (PDT-38): "Accounts" with the first three as its parts.
    let accounts = NewTicket {
        title: "Accounts".into(),
        description: Some("Sign in, stay signed in, reset a password: one PR each.".into()),
        stage: Some(InProgress),
        before: Some(ids[0].clone()),
        ..Default::default()
    };
    let accounts = accounts.validate().context("demo parent")?;
    let parent = store.create_ticket(&uuid::Uuid::now_v7().to_string(), project_id, &accounts, now).await?.summary.id;
    for id in &ids[..3] {
        let patch = pdt_core::ticket::ValidPatch { parent: Some(Some(parent.clone())), ..Default::default() };
        store.patch_ticket(id, &patch, now).await?;
    }
    // The login form is being worked on in the feature worktree.
    let attempt = uuid::Uuid::now_v7().to_string();
    store.insert_attempt(&attempt, &ids[0], Some(&wt.id), &wt.branch, "main", &wt.base_commit, now - 5 * HOUR).await?;
    Ok(())
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
        assert_eq!(sessions.len(), 5);
        let conv = sessions.iter().find(|s| s.mode == pdt_core::conversation::SessionMode::Conversation).unwrap();
        let events = store.list_session_events(&conv.id, 0, 100).await.unwrap();
        assert_eq!(events.len(), 11, "a demo conversation with one finished turn and its diff");
        assert!(sessions.iter().all(|s| s.status == SessionStatus::Finished && s.ended_at.is_some()));
        assert!(sessions.iter().any(|s| s.worktree_id.as_deref() == Some(wts[0].id.as_str())));
        assert!(store.list_sessions_by_status(SessionStatus::Interrupted).await.unwrap().is_empty());
        let workspaces = store.list_workspaces().await.unwrap();
        let tabs = |name: &str| workspaces.iter().find(|w| w.name == name).map(|w| w.tabs.len());
        assert_eq!((tabs("Sample App"), tabs("Sample Library")), (Some(5), Some(1)), "every session has a tab");

        let runs = store.list_runs(&app.id).await.unwrap();
        assert_eq!(runs.len(), 3);
        assert!(store.list_active_runs().await.unwrap().is_empty());
        let tickets = store
            .query_tickets(
                &pdt_core::ticket::TicketQuery { project: Some(app.id.clone()), ..Default::default() }
                    .validate()
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(tickets.total, 11);
        let parts: Vec<&str> = tickets.items.iter().filter(|t| t.parent.is_some()).map(|t| t.title.as_str()).collect();
        assert_eq!(parts.len(), 3, "Accounts has three parts: {parts:?}");
        let with_areas = tickets.items.iter().filter(|t| !t.areas.is_empty()).count();
        assert_eq!(with_areas, 5);
        // The login form has an active attempt on the feature worktree, whose changes are in its areas' files.
        let active = store.list_active_attempts().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].attempt.branch, "feature/login-form");
        let files = pdt_git::changed_files(Path::new(active[0].worktree_path.as_ref().unwrap()), "main", "x").unwrap();
        assert_eq!(files, ["login.css", "login.html"]);

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
