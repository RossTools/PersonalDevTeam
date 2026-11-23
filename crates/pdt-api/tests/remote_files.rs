//! Files, Changes and overlaps for remote attempts (PDT-635): an owner and a remote instance on loopback
//! ([`crate::remote_attempts`]). The owner reads and writes a remote attempt's worktree, and the remote project's
//! checkout, over the link; each Files request is one call, Changes come whole (both sides of every file) and are kept
//! until the diff changes, and the remote instance lists changed files for the owner's overlap check. Paths stay inside
//! the remote checkout. The link gets a tailnet's latency, and the timings are printed.

use crate::common::{call, get_bytes, git};
use crate::remote_attempts::{Pair, get_ticket, paired, ticket, worktrees};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A round trip on a tailnet between two machines at home, give or take.
const LATENCY: Duration = Duration::from_millis(40);

/// Starts a `windows` ticket (placed on the remote instance) without a session: its worktree's id and folder there.
async fn remote_attempt(p: &Pair, title: &str) -> (String, String, PathBuf) {
    let t = ticket(p, title, &["windows"]).await;
    let start = json!({"agent": "claude", "noSession": true});
    let (s, started) = call(&p.owner.app, "POST", &format!("/api/tickets/{t}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let branch = started["attempt"]["branch"].as_str().unwrap().to_string();
    let wt = worktrees(p).await.into_iter().find(|w| w["branch"] == branch.as_str()).unwrap();
    assert_eq!(wt["instance"]["name"], "win-box", "{wt}");
    let dir = p.checkout.join(".worktrees").join(&branch);
    assert!(dir.is_dir());
    (t, wt["id"].as_str().unwrap().to_string(), dir)
}

/// The owner's link calls since `from`.
fn calls(p: &Pair, from: u64) -> u64 {
    p.owner.state.link.calls() - from
}

#[tokio::test]
async fn the_files_view_reads_and_writes_a_remote_worktree_over_the_link() {
    let p = paired().await;
    let (_, wt, dir) = remote_attempt(&p, "Edit on win-box").await;
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "hello\n").unwrap();
    let files = format!("/api/projects/{}/files", p.project);
    p.owner.state.link.set_latency(LATENCY);
    let before = p.owner.state.link.calls();

    let t = Instant::now();
    let (s, v) = call(&p.owner.app, "GET", &format!("{files}?worktreeId={wt}"), None).await;
    let list_ms = t.elapsed().as_millis();
    assert_eq!(s, StatusCode::OK, "{v}");
    let names: Vec<&str> = v.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["src", "notes.txt"], "{v}");
    assert_eq!(v[1]["status"], "??", "{v}");
    let t = Instant::now();
    let (s, f) = call(&p.owner.app, "GET", &format!("{files}/content?worktreeId={wt}&path=notes.txt"), None).await;
    let read_ms = t.elapsed().as_millis();
    assert_eq!((s, f["text"].as_str()), (StatusCode::OK, Some("hello\n")), "{f}");
    assert_eq!(calls(&p, before), 2, "one call each");

    // Saving checks the hash there: a stale one is refused as it would be locally.
    let save = json!({"worktreeId": wt, "path": "notes.txt", "text": "hi there\n", "baseHash": f["hash"]});
    let t = Instant::now();
    let (s, v) = call(&p.owner.app, "PUT", &format!("{files}/content"), Some(save.clone())).await;
    let save_ms = t.elapsed().as_millis();
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "hi there\n");
    let (s, v) = call(&p.owner.app, "PUT", &format!("{files}/content"), Some(save)).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::CONFLICT, Some("file_changed")), "{v}");
    let new = json!({"worktreeId": wt, "path": "docs/new.md", "text": "# New\n"});
    let (s, v) = call(&p.owner.app, "PUT", &format!("{files}/content"), Some(new)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let mv = json!({"worktreeId": wt, "from": "docs/new.md", "to": "docs/moved.md"});
    let (s, v) = call(&p.owner.app, "POST", &format!("{files}/rename"), Some(mv)).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert!(dir.join("docs/moved.md").is_file() && !dir.join("docs/new.md").exists());
    let (s, v) =
        call(&p.owner.app, "DELETE", &format!("{files}/content?worktreeId={wt}&path=docs/moved.md"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{v}");
    assert!(!dir.join("docs/moved.md").exists());

    // Find and search run there.
    let (s, v) = call(&p.owner.app, "GET", &format!("{files}/find?worktreeId={wt}&q=main"), None).await;
    assert_eq!((s, v), (StatusCode::OK, json!(["src/main.rs"])));
    let (s, v) = call(&p.owner.app, "GET", &format!("{files}/search?worktreeId={wt}&q=there"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["files"][0]["path"], "notes.txt", "{v}");

    // Downloads stream through: a file, and a folder as a .zip.
    let (s, h, bytes) = get_bytes(&p.owner.app, &format!("{files}/download?worktreeId={wt}&path=src/main.rs")).await;
    assert_eq!((s, bytes.as_slice()), (StatusCode::OK, b"fn main() {}\n".as_slice()));
    assert!(h["content-disposition"].to_str().unwrap().contains("main.rs"), "{h:?}");
    assert_eq!(h["x-content-type-options"], "nosniff");
    let (s, h, bytes) = get_bytes(&p.owner.app, &format!("{files}/download?worktreeId={wt}&path=src&zip=1")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "application/zip");
    assert!(bytes.starts_with(b"PK"), "a zip");
    let (s, _, _) = get_bytes(&p.owner.app, &format!("{files}/download?worktreeId={wt}&path=src&zip=1&check=1")).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // The remote project's checkout, by instance.
    let (s, v) = call(&p.owner.app, "GET", &format!("{files}?instanceId={}", p.instance_id), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v.as_array().unwrap().iter().all(|e| e["name"] != ".worktrees" && e["name"] != ".git"), "{v}");
    let (s, v) = call(&p.owner.app, "GET", &format!("{files}?instanceId=nowhere"), None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");

    eprintln!(
        "remote Files over a {} ms link: list {list_ms} ms, read {read_ms} ms, save {save_ms} ms",
        LATENCY.as_millis()
    );

    // A dropped link says so.
    p.owner.state.link.set_offline(true);
    let (s, v) = call(&p.owner.app, "GET", &format!("{files}?worktreeId={wt}"), None).await;
    assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_GATEWAY, Some("link_unreachable")), "{v}");
}

#[tokio::test]
async fn paths_stay_inside_the_remote_checkout() {
    let p = paired().await;
    let (_, wt, dir) = remote_attempt(&p, "Stay inside").await;
    let files = format!("/api/projects/{}/files", p.project);
    let outside = |v: &Value| v["code"] == "path_outside";
    for path in ["../secret.txt", "..%2F..%2Fsecret.txt", ".git/config", ".GIT/config", "C:%5Cx"] {
        let (s, v) = call(&p.owner.app, "GET", &format!("{files}/content?worktreeId={wt}&path={path}"), None).await;
        assert!(s == StatusCode::BAD_REQUEST && outside(&v), "{path}: {s} {v}");
    }
    let save = json!({"worktreeId": wt, "path": "a/.git/hooks/pre-commit", "text": "x"});
    let (s, v) = call(&p.owner.app, "PUT", &format!("{files}/content"), Some(save)).await;
    assert!(s == StatusCode::BAD_REQUEST && outside(&v), "{s} {v}");

    // The remote instance checks again: a request the owner didn't clean is refused there too.
    #[cfg(unix)]
    {
        let secret = p.remote.tmp.path().join("secret.txt");
        std::fs::write(&secret, "secret\n").unwrap();
        std::os::unix::fs::symlink(&secret, dir.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(p.remote.tmp.path(), dir.join("up")).unwrap();
        for path in ["link.txt", "up/secret.txt"] {
            let (s, v) = call(&p.owner.app, "GET", &format!("{files}/content?worktreeId={wt}&path={path}"), None).await;
            assert!(s == StatusCode::BAD_REQUEST && outside(&v), "{path}: {s} {v}");
        }
        let (s, _, bytes) = get_bytes(&p.owner.app, &format!("{files}/download?worktreeId={wt}&path=link.txt")).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&bytes));
    }

    // A worktree folder the remote instance finds swapped for a link out isn't read.
    #[cfg(unix)]
    {
        let elsewhere = p.remote.tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("x.txt"), "x\n").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        let (s, v) = call(&p.owner.app, "GET", &format!("{files}?worktreeId={wt}"), None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("worktree_path_unsafe")), "{v}");
        let (s, v) = call(&p.owner.app, "GET", &format!("/api/worktrees/{wt}/changes"), None).await;
        assert_eq!((s, v["code"].as_str()), (StatusCode::BAD_REQUEST, Some("worktree_path_unsafe")), "{v}");
    }
    let _ = dir;
}

#[tokio::test]
async fn changes_of_a_remote_attempt_come_whole_and_are_kept_until_they_change() {
    let p = paired().await;
    let (t, wt, dir) = remote_attempt(&p, "Many files").await;
    for n in 0..8 {
        std::fs::write(dir.join(format!("f{n}.txt")), format!("file {n}\n")).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "files"]);
    std::fs::write(dir.join("f0.txt"), "file 0, edited\n").unwrap();
    p.owner.state.link.set_latency(LATENCY);
    let changes = format!("/api/worktrees/{wt}/changes");
    let before = p.owner.state.link.calls();

    // Opening Changes and reading every file: one round trip.
    let t0 = Instant::now();
    let (s, c) = call(&p.owner.app, "GET", &changes, None).await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let files = c["files"].as_array().unwrap().clone();
    assert_eq!(files.len(), 8, "{c}");
    assert_eq!(c["dirty"], true);
    for f in &files {
        let path = f["path"].as_str().unwrap();
        let (s, v) = call(&p.owner.app, "GET", &format!("{changes}/file?path={path}"), None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let n = &path[1..2];
        let want = if n == "0" { "file 0, edited\n".to_string() } else { format!("file {n}\n") };
        assert_eq!((v["old"].clone(), v["new"].as_str()), (Value::Null, Some(want.as_str())), "{v}");
    }
    let open_ms = t0.elapsed().as_millis();
    assert_eq!(calls(&p, before), 1, "the diff and all its files in one call");

    // A poll of an unchanged diff: one small call, nothing read again.
    let t0 = Instant::now();
    let (_, again) = call(&p.owner.app, "GET", &changes, None).await;
    let poll_ms = t0.elapsed().as_millis();
    assert_eq!(again["files"], c["files"]);
    assert_eq!(calls(&p, before), 2);

    // An edit there: a new version, and its new side.
    std::fs::write(dir.join("f1.txt"), "file 1, edited\n").unwrap();
    let (_, c2) = call(&p.owner.app, "GET", &changes, None).await;
    let f1 = c2["files"].as_array().unwrap().iter().find(|f| f["path"] == "f1.txt").unwrap().clone();
    let was = files.iter().find(|f| f["path"] == "f1.txt").unwrap();
    assert_ne!(f1["version"], was["version"]);
    let (_, v) = call(&p.owner.app, "GET", &format!("{changes}/file?path=f1.txt"), None).await;
    assert_eq!(v["new"], "file 1, edited\n");
    assert_eq!(calls(&p, before), 3);

    // For comparison: one call per file, as the view would make without the bundle.
    p.owner.state.exec.forget_changes();
    let t0 = Instant::now();
    for f in &files {
        let path = f["path"].as_str().unwrap();
        call(&p.owner.app, "GET", &format!("{changes}/file?path={path}"), None).await;
    }
    let per_file_ms = t0.elapsed().as_millis();
    eprintln!(
        "remote Changes over a {} ms link, 8 files: open and read all {open_ms} ms (1 call), unchanged poll \
         {poll_ms} ms, one call per file {per_file_ms} ms",
        LATENCY.as_millis()
    );

    // Merge preview: checked there (a remote branch merges through its pull request).
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "edits"]);
    let attempt = get_ticket(&p, &t).await["attempt"]["id"].as_str().unwrap().to_string();
    let (s, m) = call(&p.owner.app, "GET", &format!("/api/attempts/{attempt}/merge"), None).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert_eq!(
        (m["method"].as_str(), m["commits"].as_u64(), m["problem"].as_str()),
        (Some("forge"), Some(2), Some("no_pr")),
        "{m}"
    );

    // Approve records the remote branch's head, so Changes shows it approved.
    let (s, v) = call(&p.owner.app, "POST", &format!("/api/attempts/{attempt}/approve"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, c3) = call(&p.owner.app, "GET", &changes, None).await;
    assert_eq!(c3["approved"], true, "{c3}");
}

#[tokio::test]
async fn a_local_and_a_remote_attempt_that_change_the_same_file_overlap() {
    let p = paired().await;
    let local = ticket(&p, "Here", &[]).await;
    let start = json!({"agent": "claude", "noSession": true});
    let (s, started) = call(&p.owner.app, "POST", &format!("/api/tickets/{local}/start"), Some(start)).await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let here = p.owner.tmp.path().join("valley/.worktrees").join(started["attempt"]["branch"].as_str().unwrap());
    let (remote, _, there) = remote_attempt(&p, "There").await;
    let remote_key = get_ticket(&p, &remote).await["key"].as_str().unwrap().to_string();

    pdt_api::refresh_overlaps(&p.owner.state).await;
    assert_eq!(get_ticket(&p, &local).await["overlap"], "none");
    std::fs::write(here.join("shared.txt"), "here\n").unwrap();
    std::fs::write(there.join("shared.txt"), "there\n").unwrap();
    let before = p.owner.state.link.calls();
    pdt_api::refresh_overlaps(&p.owner.state).await;
    assert!(calls(&p, before) >= 1, "the remote instance listed its files");
    let t = get_ticket(&p, &local).await;
    assert_eq!(t["overlap"], "overlap", "{t}");
    assert_eq!(
        (t["overlaps"][0]["key"].as_str(), &t["overlaps"][0]["files"]),
        (Some(remote_key.as_str()), &json!(["shared.txt"]))
    );

    // A dropped link keeps what was last listed, rather than clearing the overlap.
    p.owner.state.link.set_offline(true);
    pdt_api::refresh_overlaps(&p.owner.state).await;
    assert_eq!(get_ticket(&p, &local).await["overlap"], "overlap");
}
