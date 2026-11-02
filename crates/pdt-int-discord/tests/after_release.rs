//! The "After a release" template (PDT-373) with its two parts on: a published stable release is announced on
//! Discord (a local fake server) and bumps a Homebrew cask in a tap by a pull request (`FakeForge`); a pre-release is
//! skipped; a dry run says what it would do. The macOS file downloads from the fake server.

use crate::common;

use axum::http::StatusCode;
use common::workflows::*;
use common::{call, git};
use pdt_forge::{Release, ReleaseAsset};
use serde_json::{Value, json};
use std::sync::Arc;

const REPO: &str = "example-org/thing";
const TAP: &str = "example-org/homebrew-tap";
/// The sha256 of what the fake server sends for the macOS file: its JSON, `"mac app bytes"`.
const SHA: &str = "7be6c85cc0d10c5de1152a7dd7560502198b42159880c998f5ac8a5bb3625011";

const CASK: &str = r#"cask "thing" do
  version "0.1.0"
  sha256 "0000"

  url "https://github.com/example-org/thing/releases/download/v#{version}/Thing.app.tar.gz"
  app "Thing.app"
end
"#;

/// The fake server: the macOS file, and Discord's webhook.
fn answer(path: &str, n: usize) -> (u16, Value) {
    if path.starts_with("/dl/") { (200, json!("mac app bytes")) } else { (200, json!({"id": format!("m{n}")})) }
}

fn release(server: &str, tag: &str, prerelease: bool) -> Release {
    Release {
        tag: tag.into(),
        name: format!("Thing {tag}"),
        url: format!("https://github.com/{REPO}/releases/tag/{tag}"),
        published_at: Some("2026-11-01T12:00:00Z".into()),
        notes: "### Added\n- a faster thing".into(),
        prerelease,
        assets: vec![
            ReleaseAsset { name: "thing-setup.exe".into(), url: format!("{server}/dl/thing-setup.exe"), size: None },
            ReleaseAsset { name: "Thing.app.tar.gz".into(), url: format!("{server}/dl/Thing.app.tar.gz"), size: None },
        ],
    }
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

fn publish(env: &Env, list: Vec<Release>) {
    env.gh.releases.lock().unwrap().insert(REPO.into(), list);
}

#[tokio::test]
async fn a_stable_release_is_announced_and_bumps_the_cask_and_a_pre_release_is_skipped() {
    let (server, seen) = fake_server(answer).await;
    let env = common::workflows::setup(vec![Arc::new(pdt_int_discord::Discord)]).await;
    let project = project(&env).await;
    connection(&env, "news", "discord_webhook", "", json!({}), Some(&format!("{server}/api/webhooks/1/abc"))).await;
    env.gh.set_file(TAP, "main", "Casks/thing.rb", CASK);
    publish(&env, vec![release(&server, "v0.1.0", false)]);

    let body = json!({"scope": "project", "projectId": project,
                      "params": {"chat": "news", "tap": TAP, "cask": "thing"}});
    let (s, w) = call(&env.app, "POST", "/api/workflow-templates/builtin:after-release/instantiate", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{w}");
    assert_eq!(w["enabled"], false, "off until the project turns it on");
    let wf = w["id"].as_str().unwrap().to_string();

    // A dry run posts and writes nothing, and says what it would.
    let payload = serde_json::to_value(pdt_core::workflow::events::ReleasePublished {
        project_id: project.clone(),
        forge: "github".into(),
        repo: REPO.into(),
        tag: "v0.3.0".into(),
        name: "Thing v0.3.0".into(),
        url: "https://example/r".into(),
        prerelease: false,
        published_at: None,
        notes: "notes".into(),
        summary: "notes".into(),
        assets: vec![pdt_core::workflow::events::ReleaseAssetRef {
            name: "Thing.dmg".into(),
            url: format!("{server}/dl/Thing.dmg"),
            size: None,
        }],
    })
    .unwrap();
    let event = json!({"type": "release.published", "payload": payload});
    let (s, d) = call(&env.app, "POST", "/api/workflows/dry-run", Some(json!({"doc": w["doc"], "event": event}))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["run"]["status"], "succeeded", "{d}");
    let would = |node: &str| step(&d["run"], node)["output"]["would"].as_str().unwrap_or_default().to_string();
    assert!(would("announce").contains("post via news"), "{d}");
    assert_eq!(
        would("cask"),
        format!("set Casks/thing.rb to 0.3.0 from Thing.dmg and open a pull request on {TAP}"),
        "{d}"
    );
    assert!(seen.all().is_empty(), "nothing downloaded or posted");
    assert!(env.gh.written.lock().unwrap().is_empty());

    let (s, _) = call(&env.app, "PATCH", &format!("/api/workflows/{wf}"), Some(json!({"enabled": true}))).await;
    assert_eq!(s, StatusCode::OK);
    pdt_api::refresh_releases(&env.state).await;

    // A pre-release: skipped by default.
    publish(&env, vec![release(&server, "v0.2.0-rc.1", true), release(&server, "v0.1.0", false)]);
    pdt_api::refresh_releases(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0]["status"], "quiet", "{r:?}");
    assert!(seen.all().is_empty(), "nothing posted for a pre-release");
    assert!(env.gh.opened.lock().unwrap().is_empty());

    // A stable release: announced, and the cask's pull request has its version and checksum.
    publish(
        &env,
        vec![
            release(&server, "v0.2.0", false),
            release(&server, "v0.2.0-rc.1", true),
            release(&server, "v0.1.0", false),
        ],
    );
    pdt_api::refresh_releases(&env.state).await;
    pdt_api::run_workflows(&env.state).await;
    let r = runs(&env, &wf).await;
    assert_eq!(r.len(), 2, "{r:?}");
    let id = r.iter().find(|x| x["trigger"]["tag"] == "v0.2.0").unwrap()["id"].as_str().unwrap().to_string();
    let run = call(&env.app, "GET", &format!("/api/workflow-runs/{id}"), None).await.1;
    assert_eq!(run["status"], "succeeded", "{run}");

    let all = seen.all();
    let posts: Vec<_> = all.iter().filter(|(m, p, _, _)| m == "POST" && p.starts_with("/api/webhooks/1/abc")).collect();
    assert_eq!(posts.len(), 1, "{all:?}");
    let content = posts[0].2["content"].as_str().unwrap();
    assert!(content.starts_with("**Thing v0.2.0 is out**\n### Added\n- a faster thing"), "{content}");
    assert!(content.ends_with(&format!("https://github.com/{REPO}/releases/tag/v0.2.0")), "{content}");
    let downloads: Vec<_> = all.iter().filter(|(_, p, _, _)| p.starts_with("/dl/")).map(|(_, p, _, _)| p).collect();
    assert_eq!(downloads, ["/dl/Thing.app.tar.gz"], "only the macOS file");

    let opened = env.gh.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1, "{opened:?}");
    assert_eq!((opened[0].head.as_str(), opened[0].base.as_str()), ("thing-0.2.0", "main"));
    assert_eq!(opened[0].title, "thing 0.2.0");
    assert!(opened[0].body.contains(SHA), "{}", opened[0].body);
    let bumped = env.gh.file_text(TAP, "thing-0.2.0", "Casks/thing.rb").unwrap();
    assert_eq!(bumped, CASK.replace("\"0.1.0\"", "\"0.2.0\"").replace("\"0000\"", &format!("\"{SHA}\"")));
    assert_eq!(env.gh.file_text(TAP, "main", "Casks/thing.rb").unwrap(), CASK, "the tap's main waits for the PR");
    let cask = &step(&run, "cask")["output"];
    assert_eq!((&cask["version"], &cask["sha256"], &cask["number"]), (&json!("0.2.0"), &json!(SHA), &json!(100)));
}
