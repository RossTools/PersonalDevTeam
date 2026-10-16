//! Run configurations (remote-host spec §4.1): the repo's optional `.pdt/project.toml`, merged field by field with
//! the host's overrides (same schema, matched by `name`). Pure.

use crate::brand::PROJECT_CONFIG_FILE;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default, rename = "run")]
    pub runs: Vec<EntryFile>,
    #[serde(default, rename = "test")]
    pub tests: Vec<EntryFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryFile {
    pub name: String,
    pub setup: Option<String>,
    pub command: Option<String>,
    pub web: Option<bool>,
    pub data: Option<bool>,
    pub env: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunDef {
    pub name: String,
    pub setup: Option<String>,
    pub command: String,
    pub web: bool,
    pub data: bool,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunConfig {
    pub runs: Vec<RunDef>,
    pub tests: Vec<RunDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    File,
    Overrides,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::File => PROJECT_CONFIG_FILE,
            Source::Overrides => "the host overrides",
        })
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RunConfigError {
    #[error("{origin} is not valid: {message}")]
    Parse { origin: Source, message: String },
    #[error("\"{0}\" has no command")]
    MissingCommand(String),
    #[error("every run and test needs a name")]
    EmptyName,
    #[error(
        "\"{0}\" cannot be a run or test name: it is also a folder name, so it cannot be `.` or `..` or contain \
         `/ \\ : < > | \" ? *` or control characters"
    )]
    BadName(String),
    #[error("the name \"{0}\" is used twice")]
    DuplicateName(String),
    #[error("\"{0}\" is not a valid environment variable name")]
    BadEnvName(String),
    #[error("test \"{0}\" cannot be a web run")]
    WebTest(String),
}

pub fn parse(text: &str, origin: Source) -> Result<ConfigFile, RunConfigError> {
    toml::from_str(text).map_err(|e| RunConfigError::Parse { origin, message: e.to_string() })
}

fn merge_list(base: Vec<EntryFile>, over: Vec<EntryFile>) -> Result<Vec<EntryFile>, RunConfigError> {
    let mut out: Vec<EntryFile> = Vec::new();
    for e in base.into_iter() {
        if out.iter().any(|o| o.name.trim() == e.name.trim()) {
            return Err(RunConfigError::DuplicateName(e.name.trim().into()));
        }
        out.push(e);
    }
    let mut seen_over: Vec<String> = Vec::new();
    for o in over {
        let name = o.name.trim().to_string();
        if seen_over.contains(&name) {
            return Err(RunConfigError::DuplicateName(name));
        }
        seen_over.push(name.clone());
        match out.iter_mut().find(|b| b.name.trim() == name) {
            Some(b) => {
                b.setup = o.setup.or(b.setup.take());
                b.command = o.command.or(b.command.take());
                b.web = o.web.or(b.web);
                b.data = o.data.or(b.data);
                if let Some(env) = o.env {
                    b.env.get_or_insert_with(BTreeMap::new).extend(env);
                }
            }
            None => out.push(o),
        }
    }
    Ok(out)
}

fn valid_env_name(n: &str) -> bool {
    let mut chars = n.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A name is also a folder name (the run's data folder), so it must be one on every OS and stay where it is put.
fn valid_name(n: &str) -> bool {
    n != "." && n != ".." && !n.chars().any(|c| c.is_control() || "/\\:<>|\"?*".contains(c))
}

fn finish(e: EntryFile, test: bool) -> Result<RunDef, RunConfigError> {
    let name = e.name.trim().to_string();
    if name.is_empty() {
        return Err(RunConfigError::EmptyName);
    }
    if !valid_name(&name) {
        return Err(RunConfigError::BadName(name));
    }
    let command =
        e.command.filter(|c| !c.trim().is_empty()).ok_or_else(|| RunConfigError::MissingCommand(name.clone()))?;
    let env = e.env.unwrap_or_default();
    if let Some(bad) = env.keys().find(|k| !valid_env_name(k)) {
        return Err(RunConfigError::BadEnvName(bad.clone()));
    }
    let web = e.web.unwrap_or(false);
    if test && web {
        return Err(RunConfigError::WebTest(name));
    }
    let setup = e.setup.filter(|s| !s.trim().is_empty());
    Ok(RunDef { name, setup, command, web, data: e.data.unwrap_or(false), env })
}

/// The effective configuration: `file` with `overrides` merged over it (either may be absent).
pub fn merge(file: Option<ConfigFile>, overrides: Option<ConfigFile>) -> Result<RunConfig, RunConfigError> {
    let file = file.unwrap_or_default();
    let over = overrides.unwrap_or_default();
    let runs = merge_list(file.runs, over.runs)?.into_iter().map(|e| finish(e, false)).collect::<Result<_, _>>()?;
    let tests = merge_list(file.tests, over.tests)?.into_iter().map(|e| finish(e, true)).collect::<Result<_, _>>()?;
    Ok(RunConfig { runs, tests })
}

/// The values PDT provides to a run (spec §4.2 "Variables PDT sets").
#[derive(Debug, Clone, PartialEq)]
pub struct RunVars {
    pub port: Option<u16>,
    pub data_dir: Option<String>,
    pub url: Option<String>,
    /// The tailnet DNS name when remote access is active, else `127.0.0.1`.
    pub host: String,
    pub branch: String,
    pub owner_login: Option<String>,
}

impl RunVars {
    /// (name, value) for every variable that has a value, in a fixed order.
    pub fn pairs(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        if let Some(p) = self.port {
            v.push(("PORT", p.to_string()));
        }
        if let Some(d) = &self.data_dir {
            v.push(("PDT_RUN_DATA", d.clone()));
        }
        if let Some(u) = &self.url {
            v.push(("PDT_RUN_URL", u.clone()));
        }
        v.push(("PDT_RUN_HOST", self.host.clone()));
        v.push(("PDT_RUN_BRANCH", self.branch.clone()));
        if let Some(o) = &self.owner_login {
            v.push(("PDT_OWNER_LOGIN", o.clone()));
        }
        v
    }
}

/// Replaces `$NAME` and `${NAME}` for the names in `vars`; anything else (other variables, a lone `$`) is left for
/// the shell, so the same text works under cmd.exe and sh.
pub fn expand(text: &str, vars: &[(&str, String)]) -> String {
    let lookup = |n: &str| vars.iter().find(|(k, _)| *k == n).map(|(_, v)| v.as_str());
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let (name, consumed) = if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => (&braced[..end], end + 2),
                None => ("", 0),
            }
        } else {
            let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
            (&after[..end], end)
        };
        match lookup(name).filter(|_| !name.is_empty()) {
            Some(v) => {
                out.push_str(v);
                rest = &after[consumed..];
            }
            None => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The variables a run's processes get: PDT's own, then the definition's `env` (expanded), which wins.
pub fn run_env(def: &RunDef, vars: &RunVars) -> Vec<(String, String)> {
    let pairs = vars.pairs();
    let mut env: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    for (k, v) in &def.env {
        env.retain(|(n, _)| n != k);
        env.push((k.clone(), expand(v, &pairs)));
    }
    env
}

/// Whether a run's processes get the run's host or link baked in (`$PDT_RUN_HOST` / `$PDT_RUN_URL` in its setup,
/// command or env), so they must restart when remote access changes it (final review #9).
pub fn uses_run_host(def: &RunDef) -> bool {
    // Any replacement differs from the `$NAME` it replaces, so a changed text means a reference.
    let probe = [("PDT_RUN_HOST", "#".to_string()), ("PDT_RUN_URL", "#".to_string())];
    let refers = |text: &str| expand(text, &probe) != text;
    def.setup.as_deref().is_some_and(refers) || refers(&def.command) || def.env.values().any(|v| refers(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_runs_use_the_run_host() {
        let def = |setup: Option<&str>, command: &str, env: &[(&str, &str)]| RunDef {
            name: "r".into(),
            setup: setup.map(String::from),
            command: command.into(),
            web: true,
            data: false,
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        };
        assert!(uses_run_host(&def(None, "x", &[("PDT_ALLOWED_HOSTS", "$PDT_RUN_HOST:$PORT")])));
        assert!(uses_run_host(&def(None, "x", &[("PUBLIC", "${PDT_RUN_URL}")])));
        assert!(uses_run_host(&def(None, "serve --host $PDT_RUN_HOST", &[])));
        assert!(uses_run_host(&def(Some("echo $PDT_RUN_URL > url.txt"), "x", &[])));
        let env = [("A", "$PDT_RUN_DATA"), ("B", "$PDT_RUN_HOSTS")];
        assert!(!uses_run_host(&def(Some("pnpm i"), "serve --port $PORT", &env)));
    }

    const FILE: &str = r#"
[[run]]
name    = "web"
setup   = "pnpm install"
command = "pnpm dev --port $PORT"
web     = true
data    = true
env     = { API_URL = "http://127.0.0.1:$PORT/api", MODE = "dev" }

[[test]]
name    = "unit"
command = "pnpm test"
"#;

    fn file() -> ConfigFile {
        parse(FILE, Source::File).unwrap()
    }

    #[test]
    fn parses_the_spec_example() {
        let c = merge(Some(file()), None).unwrap();
        let web = &c.runs[0];
        assert_eq!((web.name.as_str(), web.web, web.data), ("web", true, true));
        assert_eq!(web.setup.as_deref(), Some("pnpm install"));
        assert_eq!(web.env["MODE"], "dev");
        assert_eq!(c.tests[0].command, "pnpm test");
        assert!(!c.tests[0].web);
    }

    #[test]
    fn overrides_merge_field_by_field_and_by_name() {
        let o = parse(
            r#"
[[run]]
name = "web"
command = "pnpm dev --host 0.0.0.0 --port $PORT"
env = { MODE = "test", EXTRA = "1" }

[[run]]
name = "cli"
command = "cargo run"
"#,
            Source::Overrides,
        )
        .unwrap();
        let c = merge(Some(file()), Some(o)).unwrap();
        let web = &c.runs[0];
        assert_eq!(web.command, "pnpm dev --host 0.0.0.0 --port $PORT");
        assert_eq!(web.setup.as_deref(), Some("pnpm install"), "kept from the file");
        assert!(web.web, "kept from the file");
        assert_eq!(web.env["MODE"], "test");
        assert_eq!(web.env["EXTRA"], "1");
        assert_eq!(web.env["API_URL"], "http://127.0.0.1:$PORT/api");
        assert_eq!(c.runs[1].name, "cli");
        assert!(!c.runs[1].web && !c.runs[1].data);
    }

    #[test]
    fn overrides_alone_can_supply_everything() {
        let o = parse("[[run]]\nname = \"a\"\ncommand = \"x\"\n", Source::Overrides).unwrap();
        assert_eq!(merge(None, Some(o)).unwrap().runs.len(), 1);
        assert_eq!(merge(None, None).unwrap(), RunConfig::default());
    }

    #[test]
    fn errors_are_specific() {
        assert!(matches!(parse("[[run]\n", Source::File), Err(RunConfigError::Parse { origin: Source::File, .. })));
        assert!(matches!(
            parse("[[run]]\nname = \"a\"\ncommand = \"x\"\nwebb = true\n", Source::Overrides),
            Err(RunConfigError::Parse { origin: Source::Overrides, .. })
        ));
        let no_cmd = parse("[[run]]\nname = \"a\"\n", Source::File).unwrap();
        assert_eq!(merge(Some(no_cmd), None), Err(RunConfigError::MissingCommand("a".into())));
        let blank = parse("[[run]]\nname = \" \"\ncommand = \"x\"\n", Source::File).unwrap();
        assert_eq!(merge(Some(blank), None), Err(RunConfigError::EmptyName));
        let dup =
            parse("[[run]]\nname = \"a\"\ncommand = \"x\"\n[[run]]\nname = \"a\"\ncommand = \"y\"\n", Source::File)
                .unwrap();
        assert_eq!(merge(Some(dup), None), Err(RunConfigError::DuplicateName("a".into())));
        let bad_env =
            parse("[[run]]\nname = \"a\"\ncommand = \"x\"\nenv = { \"A=B\" = \"1\" }\n", Source::File).unwrap();
        assert_eq!(merge(Some(bad_env), None), Err(RunConfigError::BadEnvName("A=B".into())));
        let web_test = parse("[[test]]\nname = \"t\"\ncommand = \"x\"\nweb = true\n", Source::File).unwrap();
        assert_eq!(merge(Some(web_test), None), Err(RunConfigError::WebTest("t".into())));
    }

    /// A name becomes a folder under the run data dir: it must not climb out of it or be unusable on any OS.
    #[test]
    fn names_that_are_not_safe_folder_names_are_rejected() {
        for bad in [".", "..", "a/b", "a\\b", "c:", "a<b", "a>b", "a|b", "a\"b", "a?b", "a*b", "a\u{1}b", "a\tb"] {
            let text = format!("[[run]]\nname = {}\ncommand = \"x\"\n", toml_string(bad));
            let cfg = parse(&text, Source::File).unwrap();
            assert_eq!(merge(Some(cfg), None), Err(RunConfigError::BadName(bad.into())), "{bad:?}");
        }
        for good in ["web", "web 2", "api.v1", "my-test_1", "é"] {
            let text = format!("[[test]]\nname = {}\ncommand = \"x\"\n", toml_string(good));
            assert!(merge(Some(parse(&text, Source::File).unwrap()), None).is_ok(), "{good:?}");
        }
    }

    fn toml_string(s: &str) -> String {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    fn vars() -> RunVars {
        RunVars {
            port: Some(8401),
            data_dir: Some("/data/runs/p/main/web".into()),
            url: Some("https://box.t.ts.net:8401/".into()),
            host: "box.t.ts.net".into(),
            branch: "feature/x".into(),
            owner_login: Some("me@example.com".into()),
        }
    }

    #[test]
    fn expansion_replaces_only_known_names() {
        let p = vars().pairs();
        assert_eq!(expand("pnpm dev --port $PORT", &p), "pnpm dev --port 8401");
        assert_eq!(expand("${PORT}x $PORTX $HOME $$ $", &p), "8401x $PORTX $HOME $$ $");
        assert_eq!(expand("$PDT_RUN_HOST:$PORT", &p), "box.t.ts.net:8401");
        assert_eq!(expand("$PDT_OWNER_LOGIN/$PDT_RUN_BRANCH", &p), "me@example.com/feature/x");
        assert_eq!(expand("${PDT_RUN_DATA}/db", &p), "/data/runs/p/main/web/db");
        assert_eq!(expand("${NOPE", &p), "${NOPE");
    }

    #[test]
    fn run_env_sets_pdt_vars_then_explicit_env_wins() {
        let c = merge(Some(file()), None).unwrap();
        let env = run_env(&c.runs[0], &vars());
        let get = |k: &str| env.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("PORT"), Some("8401"));
        assert_eq!(get("PDT_RUN_URL"), Some("https://box.t.ts.net:8401/"));
        assert_eq!(get("PDT_RUN_HOST"), Some("box.t.ts.net"));
        assert_eq!(get("PDT_RUN_BRANCH"), Some("feature/x"));
        assert_eq!(get("PDT_RUN_DATA"), Some("/data/runs/p/main/web"));
        assert_eq!(get("PDT_OWNER_LOGIN"), Some("me@example.com"));
        assert_eq!(get("API_URL"), Some("http://127.0.0.1:8401/api"));
        let mut v = vars();
        v.port = None;
        v.data_dir = None;
        v.url = None;
        v.owner_login = None;
        let env = run_env(&c.tests[0], &v);
        assert!(env.iter().all(|(k, _)| k != "PORT" && k != "PDT_RUN_DATA" && k != "PDT_OWNER_LOGIN"));
    }

    #[test]
    fn explicit_env_overrides_a_pdt_variable() {
        let o = parse("[[run]]\nname = \"a\"\ncommand = \"x\"\nenv = { PDT_RUN_HOST = \"custom\" }\n", Source::File)
            .unwrap();
        let c = merge(Some(o), None).unwrap();
        let env = run_env(&c.runs[0], &vars());
        assert_eq!(env.iter().rev().find(|(k, _)| k == "PDT_RUN_HOST").unwrap().1, "custom");
    }

    #[test]
    fn pdts_own_configuration_is_valid() {
        let c = merge(Some(parse(include_str!("../../../.pdt/project.toml"), Source::File).unwrap()), None).unwrap();
        let pdt = &c.runs[0];
        assert!(pdt.web && pdt.data);
        assert_eq!(pdt.env["PDT_PORT"], "$PORT");
        assert_eq!(pdt.env["PDT_DATA_DIR"], "$PDT_RUN_DATA");
        assert_eq!(pdt.env["PDT_PROFILE"], "dev");
        // PDT supplies the owner itself; repeating it would pass the literal "$PDT_OWNER_LOGIN" when there is none.
        assert!(!pdt.env.contains_key("PDT_OWNER_LOGIN"));
        // The first word must be a command under cmd.exe too, so not a path with forward slashes.
        assert!(!pdt.command.split_whitespace().next().unwrap().contains('/'));
        let env = run_env(pdt, &vars());
        let get = |k: &str| env.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap();
        assert_eq!(get("PDT_PORT"), "8401");
        assert_eq!(get("PDT_ALLOWED_HOSTS"), "box.t.ts.net:8401");
        assert_eq!(c.tests.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["rust", "web"]);
    }
}
