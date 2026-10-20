mod common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::{AppState, router};
use pdt_store::Store;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Env {
    app: axum::Router,
    state: AppState,
    repo: PathBuf,
    pid: String,
    tmp: tempfile::TempDir,
}

fn init_repo(repo: &Path) {
    std::fs::create_dir_all(repo).unwrap();
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-q", "-m", "init"]);
}

async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let state = AppState::new(store, tmp.path().join("data"));
    let app = router(state.clone(), None);
    let (s, p) = call(
        &app,
        "POST",
        "/api/projects",
        Some(json!({"name": "Personal Dev Team", "repoPath": repo.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { pid: p["id"].as_str().unwrap().into(), app, state, repo, tmp }
}

#[tokio::test]
async fn projects_get_keys_that_can_be_changed() {
    let env = setup().await;
    let (_, list) = call(&env.app, "GET", "/api/projects", None).await;
    assert_eq!(list[0]["key"], "PDT");
    assert_eq!(list[0]["githubRepo"], Value::Null);

    let other = env.tmp.path().join("other");
    init_repo(&other);
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects",
        Some(json!({"name": "x", "repoPath": other.to_string_lossy(), "key": "pdt"})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("key_taken")));
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects",
        Some(json!({"name": "x", "repoPath": other.to_string_lossy(), "key": "no good"})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects",
        Some(json!({"name": "x", "repoPath": other.to_string_lossy(), "key": "oth"})),
    )
    .await;
    assert_eq!((s, v["key"].as_str()), (StatusCode::CREATED, Some("OTH")));

    let uri = format!("/api/projects/{}", env.pid);
    let (s, v) = call(&env.app, "PATCH", &uri, Some(json!({"name": " PDT ", "key": "core"}))).await;
    assert_eq!((s, v["name"].as_str(), v["key"].as_str()), (StatusCode::OK, Some("PDT"), Some("CORE")));
    let (s, v) = call(&env.app, "PATCH", &uri, Some(json!({"key": "OTH"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("key_taken")));
    let (s, _) = call(&env.app, "PATCH", &uri, Some(json!({"name": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "PATCH", "/api/projects/nope", Some(json!({"name": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, ws) = call(&env.app, "GET", "/api/workspaces", None).await;
    assert_eq!(ws[0]["name"], "PDT", "the project's workspace is renamed with it");
}

#[tokio::test]
async fn base_branch_defaults_to_the_default_branch_and_can_be_changed() {
    let env = setup().await;
    // A clone checked out on a feature branch: the base is origin's default branch, not the checked-out one.
    let clone = env.tmp.path().join("clone");
    git(env.tmp.path(), &["clone", "-q", &env.repo.to_string_lossy(), &clone.to_string_lossy()]);
    git(&clone, &["checkout", "-q", "-b", "feature/x"]);
    let (s, p) =
        call(&env.app, "POST", "/api/projects", Some(json!({"name": "c", "repoPath": clone.to_string_lossy()}))).await;
    assert_eq!((s, p["baseBranch"].as_str()), (StatusCode::CREATED, Some("main")), "{p}");

    // Any local branch or one only on origin; nothing else.
    git(&env.repo, &["branch", "release"]);
    git(&clone, &["fetch", "-q"]);
    let uri = format!("/api/projects/{}", p["id"].as_str().unwrap());
    let (s, v) = call(&env.app, "PATCH", &uri, Some(json!({"baseBranch": " feature/x "}))).await;
    assert_eq!((s, v["baseBranch"].as_str()), (StatusCode::OK, Some("feature/x")), "{v}");
    let (s, v) = call(&env.app, "PATCH", &uri, Some(json!({"baseBranch": "release"}))).await;
    assert_eq!((s, v["baseBranch"].as_str()), (StatusCode::OK, Some("release")), "{v}");
    let (s, v) = call(&env.app, "PATCH", &uri, Some(json!({"baseBranch": "nope"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("unknown_branch")));
    let (s, _) = call(&env.app, "PATCH", &uri, Some(json!({"baseBranch": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "PATCH", &uri, Some(json!({"baseBranch": "-x"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (_, v) = call(&env.app, "GET", &uri, None).await;
    assert_eq!(v["baseBranch"], "release");
}

#[tokio::test]
async fn github_origins_are_recognised() {
    let env = setup().await;
    git(&env.repo, &["remote", "add", "origin", "git@github.com:ross-game-tools/PersonalDevTeam.git"]);
    let (_, p) = call(&env.app, "GET", &format!("/api/projects/{}", env.pid), None).await;
    assert_eq!(p["githubRepo"], "ross-game-tools/PersonalDevTeam");
}

#[tokio::test]
async fn removing_a_project_stops_its_sessions_and_keeps_its_files() {
    let env = setup().await;
    let (s, sess) = call(
        &env.app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": env.pid, "kind": "shell", "cols": 80, "rows": 24})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{sess}");
    let sid = sess["id"].as_str().unwrap().to_string();
    let (s, _) = call(&env.app, "DELETE", &format!("/api/projects/{}", env.pid), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(!env.state.ptys.is_running(&sid));
    let (_, list) = call(&env.app, "GET", "/api/projects", None).await;
    assert_eq!(list, json!([]));
    let (_, ws) = call(&env.app, "GET", "/api/workspaces", None).await;
    assert_eq!(ws, json!([]));
    assert!(env.repo.join("README.md").is_file(), "files are never touched");
    let (s, _) = call(&env.app, "DELETE", &format!("/api/projects/{}", env.pid), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn clone_by_url_adds_the_project() {
    let env = setup().await;
    let parent = env.tmp.path().join("projects");
    let (s, p) = call(
        &env.app,
        "POST",
        "/api/projects/clone",
        Some(json!({"url": env.repo.to_string_lossy(), "parentDir": parent.to_string_lossy()})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    assert_eq!(p["name"], "repo");
    assert_eq!(p["key"], "REPO");
    assert!(parent.join("repo").join("README.md").is_file());

    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects/clone",
        Some(json!({"url": env.repo.to_string_lossy(), "parentDir": parent.to_string_lossy()})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("path_exists")));
    let missing = env.tmp.path().join("nothing-here");
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects/clone",
        Some(json!({"url": missing.to_string_lossy(), "parentDir": parent.to_string_lossy()})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("clone_failed")), "{v}");
    assert!(!parent.join("nothing-here").exists());
    let typo = env.tmp.path().join("no-such-dir").join("work");
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects/clone",
        Some(json!({"url": env.repo.to_string_lossy(), "parentDir": typo.to_string_lossy()})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("folder_missing")), "{v}");
    assert!(!typo.exists(), "a mistyped folder is not created");
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects/clone",
        Some(json!({"url": env.repo.to_string_lossy(), "parentDir": "relative/dir"})),
    )
    .await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_folder")), "{v}");
    let fresh = env.tmp.path().join("fresh");
    let (s, v) = call(
        &env.app,
        "POST",
        "/api/projects/clone",
        Some(json!({"url": env.repo.to_string_lossy(), "parentDir": fresh.to_string_lossy(), "name": "Second"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "a missing folder whose parent exists is created: {v}");
    let (s, v) = call(&env.app, "POST", "/api/projects/clone", Some(json!({"url": "--upload-pack=x"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_input")));
}

async fn worktree(env: &Env, branch: &str) -> Value {
    let (s, w) =
        call(&env.app, "POST", &format!("/api/projects/{}/worktrees", env.pid), Some(json!({"branch": branch}))).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    w
}

#[tokio::test]
async fn worktrees_made_outside_pdt_are_adopted() {
    let env = setup().await;
    let path = env.repo.join(".worktrees").join("by-hand");
    git(&env.repo, &["worktree", "add", "-q", "-b", "by-hand", &path.to_string_lossy()]);
    let w = worktree(&env, "by-hand").await;
    assert_eq!(w["baseCommit"].as_str().unwrap().len(), 40);
    // A folder that is not a worktree for that branch still fails.
    std::fs::create_dir_all(env.repo.join(".worktrees").join("junk")).unwrap();
    let (s, _) =
        call(&env.app, "POST", &format!("/api/projects/{}/worktrees", env.pid), Some(json!({"branch": "junk"}))).await;
    assert_ne!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn removing_worktrees() {
    let env = setup().await;
    let w = worktree(&env, "feat").await;
    let (id, path) = (w["id"].as_str().unwrap().to_string(), PathBuf::from(w["path"].as_str().unwrap()));
    let uri = format!("/api/worktrees/{id}");

    // Busy while a session runs in it.
    let (_, sess) = call(
        &env.app,
        "POST",
        "/api/sessions",
        Some(json!({"projectId": env.pid, "worktreeId": id, "kind": "shell", "cols": 80, "rows": 24})),
    )
    .await;
    let sid = sess["id"].as_str().unwrap().to_string();
    let (s, v) = call(&env.app, "DELETE", &uri, None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("worktree_busy")));
    call(&env.app, "POST", &format!("/api/sessions/{sid}/kill"), None).await;
    for _ in 0..400 {
        if !env.state.ptys.is_running(&sid) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!env.state.ptys.is_running(&sid), "the killed shell never exited");

    // Dirty: refused without force.
    std::fs::write(path.join("wip.txt"), "x").unwrap();
    let (s, v) = call(&env.app, "DELETE", &uri, None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("worktree_dirty")));
    let (s, _) = call(&env.app, "DELETE", &format!("{uri}?force=true&deleteBranch=true"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(!path.exists());
    assert!(!pdt_git::branch_exists(&env.repo, "feat"));
    let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await;
    assert_eq!(list, json!([]));
    let (s, _) = call(&env.app, "GET", &format!("/api/sessions/{sid}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "its sessions go with it");

    // A worktree deleted outside PDT shows as missing and is just forgotten; its branch stays by default.
    let w = worktree(&env, "gone").await;
    let path = PathBuf::from(w["path"].as_str().unwrap());
    std::fs::remove_dir_all(&path).unwrap();
    let (_, list) = call(&env.app, "GET", &format!("/api/projects/{}/worktrees", env.pid), None).await;
    assert_eq!(list[0]["missing"], true);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/worktrees/{}", w["id"].as_str().unwrap()), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(pdt_git::branch_exists(&env.repo, "gone"));
    env.state.ptys.shutdown().await;
}

#[tokio::test]
async fn the_folder_view_lists_files_and_reads_them() {
    let env = setup().await;
    std::fs::create_dir_all(env.repo.join("src")).unwrap();
    std::fs::create_dir_all(env.repo.join("build")).unwrap();
    std::fs::write(env.repo.join(".gitignore"), "build/\n").unwrap();
    std::fs::write(env.repo.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(env.repo.join("build").join("out"), "x").unwrap();
    std::fs::write(env.repo.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
    std::fs::write(env.repo.join("big.txt"), "x".repeat(1024 * 1024 + 1)).unwrap();
    let base = format!("/api/projects/{}/files", env.pid);

    let (s, list) = call(&env.app, "GET", &base, None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["src", ".gitignore", "README.md", "big.txt", "blob.bin"]);
    assert_eq!(list[0]["dir"], true);
    assert_eq!(list[0]["status"], "M");
    let (_, src) = call(&env.app, "GET", &format!("{base}?path=src"), None).await;
    assert_eq!(src[0]["path"], "src/main.rs");
    assert_eq!(src[0]["status"], "??");

    let (s, f) = call(&env.app, "GET", &format!("{base}/content?path=src/main.rs"), None).await;
    assert_eq!((s, f["text"].as_str()), (StatusCode::OK, Some("fn main() {}\n")));
    let (_, f) = call(&env.app, "GET", &format!("{base}/content?path=blob.bin"), None).await;
    assert_eq!((f["binary"].as_bool(), f["text"].clone()), (Some(true), Value::Null));
    let (_, f) = call(&env.app, "GET", &format!("{base}/content?path=big.txt"), None).await;
    assert_eq!((f["tooLarge"].as_bool(), f["text"].clone()), (Some(true), Value::Null));

    for bad in ["../repo/README.md", ".git/config", "src/../../x"] {
        let (s, v) = call(&env.app, "GET", &format!("{base}/content?path={bad}"), None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("path_outside")), "{bad}");
    }
    let (s, _) = call(&env.app, "GET", &format!("{base}/content?path=nope.txt"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "GET", &format!("{base}?worktreeId=nope"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // A symlink out of the checkout is refused.
    #[cfg(unix)]
    {
        let outside = env.tmp.path().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, env.repo.join("link.txt")).unwrap();
        let (s, v) = call(&env.app, "GET", &format!("{base}/content?path=link.txt"), None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("path_outside")));
    }
}

#[tokio::test]
async fn pull_fast_forwards_the_base_branch_from_origin() {
    let env = setup().await;
    let uri = format!("/api/projects/{}/pull", env.pid);
    let (s, v) = call(&env.app, "POST", &uri, None).await;
    assert_eq!(
        (s, v["outcome"].as_str(), v["branch"].as_str()),
        (StatusCode::OK, Some("no_remote"), Some("main")),
        "{v}"
    );

    // The project's repo gets an origin that moves on (as after a PR merges).
    let upstream = env.tmp.path().join("upstream");
    git(env.tmp.path(), &["clone", "-q", "--bare", &env.repo.to_string_lossy(), &upstream.to_string_lossy()]);
    git(&env.repo, &["remote", "add", "origin", &upstream.to_string_lossy()]);
    let (s, v) = call(&env.app, "POST", &uri, Some(json!({}))).await;
    assert_eq!((s, v["outcome"].as_str()), (StatusCode::OK, Some("up_to_date")), "{v}");
    let other = env.tmp.path().join("other");
    git(env.tmp.path(), &["clone", "-q", &upstream.to_string_lossy(), &other.to_string_lossy()]);
    std::fs::write(other.join("merged.txt"), "merged\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "merged work"]);
    git(&other, &["push", "-q", "origin", "main"]);
    let (s, v) = call(&env.app, "POST", &uri, Some(json!({"branch": "main"}))).await;
    assert_eq!((s, v["outcome"].as_str()), (StatusCode::OK, Some("updated")), "{v}");
    assert!(env.repo.join("merged.txt").exists());

    let (s, v) = call(&env.app, "POST", &uri, Some(json!({"branch": "nope"}))).await;
    assert_eq!((s, v["outcome"].as_str()), (StatusCode::OK, Some("no_branch")), "{v}");
    let (s, v) = call(&env.app, "POST", &uri, Some(json!({"branch": "--help"}))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_branch")), "{v}");
    let (s, _) = call(&env.app, "POST", "/api/projects/nope/pull", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
