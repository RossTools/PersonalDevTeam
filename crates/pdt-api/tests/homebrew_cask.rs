//! `github.homebrew_cask` (PDT-373) on its own: a push to the tap's branch, a cask already at the version left alone,
//! and a pull request a second run finds instead of opening again. The tap is `FakeForge`'s; the macOS file downloads
//! from a local fake server. The "After a release" template around it is tested in pdt-int-discord.

use crate::common;

use common::workflows::*;
use serde_json::{Value, json};

const TAP: &str = "example-org/homebrew-tap";
/// The sha256 of what the fake server sends for the macOS file: its JSON, `"mac app bytes"`.
const SHA: &str = "7be6c85cc0d10c5de1152a7dd7560502198b42159880c998f5ac8a5bb3625011";

const CASK: &str = "cask \"thing\" do\n  version \"0.1.0\"\n  sha256 \"0000\"\n  url \"https://x/v#{version}/Thing.app.tar.gz\"\nend\n";

fn answer(_: &str, _: usize) -> (u16, Value) {
    (200, json!("mac app bytes"))
}

#[tokio::test]
async fn a_cask_can_be_pushed_and_one_already_there_is_left_alone() {
    let (server, _seen) = fake_server(answer).await;
    let env = common::workflows::setup(vec![]).await;
    env.gh.set_file(TAP, "main", "Casks/thing.rb", CASK);
    let cask = |push: bool| {
        manual_then(json!({"id": "cask", "kind": "github.homebrew_cask", "version": 1, "config": {
            "tap": TAP, "cask": "thing", "version": "v0.2.0", "push": push,
            "assets": [{"name": "Thing.app.tar.gz", "url": format!("{server}/dl/Thing.app.tar.gz")}]}}))
    };
    let wf = create(&env, cask(true)).await;
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(step(&r, "cask")["port"], "updated", "{r}");
    let main = env.gh.file_text(TAP, "main", "Casks/thing.rb").unwrap();
    assert!(main.contains("version \"0.2.0\"") && main.contains(SHA), "{main}");
    assert!(env.gh.opened.lock().unwrap().is_empty(), "pushed, no pull request");

    // Run again: the cask has it already.
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(step(&r, "cask")["port"], "unchanged", "{r}");
    assert_eq!(env.gh.written.lock().unwrap().len(), 1);

    // A pull request run twice finds its own.
    env.gh.set_file(TAP, "main", "Casks/thing.rb", CASK);
    let wf = create(&env, cask(false)).await;
    run_now(&env, &wf, json!({})).await;
    let first = env.gh.opened.lock().unwrap().clone();
    assert_eq!(first.len(), 1);
    let r = run_now(&env, &wf, json!({})).await;
    assert_eq!(r["status"], "succeeded", "{r}");
    assert_eq!(env.gh.opened.lock().unwrap().len(), 1, "not opened twice");
    assert_eq!(step(&r, "cask")["output"]["number"], 100);
}
