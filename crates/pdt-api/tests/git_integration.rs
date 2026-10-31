use crate::common;

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
    git(&env.repo, &["remote", "add", "origin", "git@github.com:example-org/PersonalDevTeam.git"]);
    let (_, p) = call(&env.app, "GET", &format!("/api/projects/{}", env.pid), None).await;
    let f = &p["forge"];
    assert_eq!((f["kind"].as_str(), f["repo"].as_str()), (Some("github"), Some("example-org/PersonalDevTeam")), "{p}");
    assert_eq!(f["repoUrl"], "https://github.com/example-org/PersonalDevTeam");
    assert_eq!(f["branchUrl"], "https://github.com/example-org/PersonalDevTeam/tree/{branch}");
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

/// Saves `text` over the file opened with `hash` (none: a new file).
async fn save(env: &Env, wt: Option<&str>, path: &str, text: &str, hash: Option<&str>) -> (StatusCode, Value) {
    let body = json!({"worktreeId": wt, "path": path, "text": text, "baseHash": hash});
    call(&env.app, "PUT", &format!("/api/projects/{}/files/content", env.pid), Some(body)).await
}

#[tokio::test]
async fn the_files_view_saves_files_and_refuses_stale_saves() {
    let env = setup().await;
    let base = format!("/api/projects/{}/files", env.pid);
    let (_, f) = call(&env.app, "GET", &format!("{base}/content?path=README.md"), None).await;
    let opened = f["hash"].as_str().unwrap().to_string();

    let (s, f) = save(&env, None, "README.md", "hello, edited\n", Some(&opened)).await;
    assert_eq!(
        (s, f["text"].as_str(), f["status"].as_str()),
        (StatusCode::OK, Some("hello, edited\n"), Some("M")),
        "{f}"
    );
    assert_eq!(std::fs::read_to_string(env.repo.join("README.md")).unwrap(), "hello, edited\n");
    let saved = f["hash"].as_str().unwrap().to_string();
    assert_ne!(saved, opened);

    // Saving again from the old text: the file changed since, so nothing is written.
    let (s, v) = save(&env, None, "README.md", "stale", Some(&opened)).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("file_changed")));
    assert_eq!(std::fs::read_to_string(env.repo.join("README.md")).unwrap(), "hello, edited\n");
    // Changed underneath (an agent wrote it): refused too.
    std::fs::write(env.repo.join("README.md"), "by the agent\n").unwrap();
    let (s, _) = save(&env, None, "README.md", "mine", Some(&saved)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    // A file that exists can't be "created".
    let (s, v) = save(&env, None, "README.md", "mine", None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("file_exists")));

    // New files (with their folders), rename, delete.
    let (s, f) = save(&env, None, "notes/todo.md", "", None).await;
    assert_eq!((s, f["status"].as_str()), (StatusCode::OK, Some("??")), "{f}");
    assert_eq!(std::fs::read_to_string(env.repo.join("notes").join("todo.md")).unwrap(), "");
    let rename = |from: &str, to: &str| json!({"from": from, "to": to});
    let (s, _) =
        call(&env.app, "POST", &format!("{base}/rename"), Some(rename("notes/todo.md", "notes/done.md"))).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(env.repo.join("notes").join("done.md").is_file() && !env.repo.join("notes").join("todo.md").exists());
    let (s, v) = call(&env.app, "POST", &format!("{base}/rename"), Some(rename("notes/done.md", "README.md"))).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("file_exists")));
    let (s, _) = call(&env.app, "POST", &format!("{base}/rename"), Some(rename("nope.md", "x.md"))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&env.app, "DELETE", &format!("{base}/content?path=notes/done.md"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(!env.repo.join("notes").join("done.md").exists());
    let (s, _) = call(&env.app, "DELETE", &format!("{base}/content?path=notes"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "folders are not deleted");

    // Binary and too-large files stay read-only.
    std::fs::write(env.repo.join("blob.bin"), [0u8, 1, 2]).unwrap();
    let (_, f) = call(&env.app, "GET", &format!("{base}/content?path=blob.bin"), None).await;
    let (s, v) = save(&env, None, "blob.bin", "text", f["hash"].as_str()).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("not_editable")));
    let (s, _) = save(&env, None, "big.txt", &"x".repeat(1024 * 1024 + 1), None).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!env.repo.join("big.txt").exists());

    // In a worktree: written there, not in the main checkout.
    let w = worktree(&env, "edit").await;
    let wid = w["id"].as_str().unwrap();
    let wpath = PathBuf::from(w["path"].as_str().unwrap());
    let (_, f) = call(&env.app, "GET", &format!("{base}/content?path=README.md&worktreeId={wid}"), None).await;
    let (s, _) = save(&env, Some(wid), "README.md", "in the worktree\n", f["hash"].as_str()).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(std::fs::read_to_string(wpath.join("README.md")).unwrap(), "in the worktree\n");
    assert_eq!(std::fs::read_to_string(env.repo.join("README.md")).unwrap(), "by the agent\n");
    assert!(pdt_git::status(&wpath, "").unwrap().iter().any(|(p, c)| p == "README.md" && c == "M"));
}

#[tokio::test]
async fn the_files_view_never_writes_outside_the_checkout_or_into_git() {
    let env = setup().await;
    let base = format!("/api/projects/{}/files", env.pid);
    let w = worktree(&env, "edit").await;
    let wid = w["id"].as_str().unwrap();
    for (wt, bad) in [
        (None, "../escape.txt"),
        (None, "src/../../escape.txt"),
        (None, ".git/config"),
        (None, ".git/hooks/pre-commit"),
        (None, "vendor/.git/config"),
        (Some(wid), ".git"),
        (None, "C:\\escape.txt"),
    ] {
        let (s, v) = save(&env, wt, bad, "x", None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("path_outside")), "{bad}");
        let (s, _) = call(
            &env.app,
            "POST",
            &format!("{base}/rename"),
            Some(json!({"worktreeId": wt, "from": "README.md", "to": bad})),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "rename to {bad}");
    }
    let (s, _) = call(&env.app, "DELETE", &format!("{base}/content?path=.git/HEAD"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(!env.tmp.path().join("escape.txt").exists());
    assert!(env.repo.join(".git").join("HEAD").is_file());

    // Symlinks that lead out: a linked file, and a linked folder for a new file.
    #[cfg(unix)]
    {
        let outside = env.tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), env.repo.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, env.repo.join("linkdir")).unwrap();
        for bad in ["link.txt", "linkdir/new.txt", "linkdir/deeper/new.txt"] {
            let (s, v) = save(&env, None, bad, "x", None).await;
            assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("path_outside")), "{bad}");
        }
        let (s, _) = call(&env.app, "DELETE", &format!("{base}/content?path=linkdir/secret.txt"), None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_to_string(outside.join("secret.txt")).unwrap(), "secret");
        assert!(!outside.join("new.txt").exists() && !outside.join("deeper").exists());
    }
}

#[tokio::test]
async fn an_agent_can_be_told_which_files_the_user_changed() {
    let env = setup().await;
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/files/tell", env.pid),
        Some(json!({"sessionId": "nope", "paths": ["README.md"]})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let session = pdt_core::session::Session {
        id: "agent".into(),
        project_id: env.pid.clone(),
        worktree_id: None,
        kind: pdt_core::session::SessionKind::AgentTui,
        agent: Some(pdt_core::session::AgentKind::Claude),
        bypass_permissions: false,
        title: "Coder".into(),
        cwd: env.repo.to_string_lossy().into(),
        status: pdt_core::session::SessionStatus::Running,
        exit_code: None,
        created_at: 1,
        ended_at: None,
        external_session_id: None,
        fresh_started_at: None,
        activity: Some(pdt_core::activity::Activity::Working),
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
    env.state.store.insert_session(&session).await.unwrap();
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/projects/{}/files/tell", env.pid),
        Some(json!({"sessionId": "agent", "paths": ["README.md"]})),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    let waiting = env.state.store.waiting_session_messages().await.unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].session_id.as_deref(), Some("agent"));
    assert!(waiting[0].by_user);
    assert!(waiting[0].text.contains("`README.md`"), "{}", waiting[0].text);
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
