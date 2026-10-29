//! Run configurations (remote-host spec §4.1): the repo's optional `.pdt/project.toml`, merged field by field with
//! the host's overrides (same schema, matched by `name`). Pure.

use crate::brand::PROJECT_CONFIG_FILE;
use crate::forge::ForgeKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// The top-level sections (and keys) this PDT understands. Any other is skipped (see [`parse`]).
const SECTIONS: [&str; 5] = ["run", "test", "area", "forge", "ci"];

/// Unknown top-level keys are skipped by serde (no `deny_unknown_fields` here, unlike the entries).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ConfigFile {
    #[serde(default, rename = "run")]
    pub runs: Vec<EntryFile>,
    #[serde(default, rename = "test")]
    pub tests: Vec<EntryFile>,
    #[serde(default, rename = "area")]
    pub areas: Vec<AreaFile>,
    /// `forge = "github"`: the project's forge (PDT-176), a forge plugin's id (PDT-241); a value that can't be one is
    /// listed in `ignored` as `forge`, like an unknown section.
    #[serde(skip)]
    pub forge: Option<ForgeKind>,
    /// `[ci]`: the command the local forge runs as CI (PDT-178).
    #[serde(default)]
    pub ci: Option<CiFile>,
    /// Top-level sections this PDT does not know (e.g. written for a newer PDT), skipped rather than refused.
    #[serde(skip)]
    pub ignored: Vec<String>,
}

/// `[ci]` as written: `command = "cargo test"`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiFile {
    pub command: Option<String>,
}

/// The project's own CI (PDT-178): the command a project on the local forge (`forge = "local"`) runs in an attempt's
/// worktree on each new commit of its branch. It passes when the command exits 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CiDef {
    pub command: String,
}

/// An `[[area]]` as written (tickets spec §3): a named part of the project, for overlap detection.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaFile {
    pub name: String,
    pub paths: Option<Vec<String>>,
    pub exclusive: Option<bool>,
}

/// A named part of the project: the files its `paths` globs match (relative to the checkout; `*` stays within a
/// folder, `**` crosses folders). Only one active ticket at a time should touch an `exclusive` area.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AreaDef {
    pub name: String,
    pub paths: Vec<String>,
    pub exclusive: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryFile {
    pub name: String,
    pub setup: Option<String>,
    pub command: Option<String>,
    pub web: Option<bool>,
    pub data: Option<bool>,
    pub detach: Option<bool>,
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
    /// The command runs outside PDT's terminals and keeps running when pdtd restarts or updates (PDT-32). Its
    /// output goes to a log file the run's tab follows; it takes no input.
    pub detach: bool,
    pub env: BTreeMap<String, String>,
}

/// `GET /api/projects/{id}/areas`: the areas the project's main checkout and the host overrides declare.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProjectAreas {
    pub areas: Vec<AreaDef>,
    /// Why there are none when the configuration is invalid.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunConfig {
    pub runs: Vec<RunDef>,
    pub tests: Vec<RunDef>,
    pub areas: Vec<AreaDef>,
    /// The forge the file or the overrides name (the overrides' wins); None: chosen from the project's `origin`.
    pub forge: Option<ForgeKind>,
    /// `[ci]`: the command the local forge runs as CI (the overrides' wins).
    pub ci: Option<CiDef>,
    /// Top-level sections of the file or the overrides that this PDT skipped because it does not know them.
    pub ignored: Vec<String>,
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
    #[error("test \"{0}\" cannot be detached")]
    DetachedTest(String),
    #[error("\"{0}\" cannot be an area name: it must be 1–{max} characters with no commas", max = crate::ticket::MAX_AREA)]
    BadAreaName(String),
    #[error("the area name \"{0}\" is used twice")]
    DuplicateArea(String),
    #[error("area \"{0}\" needs at least one path")]
    NoAreaPaths(String),
    #[error("area \"{area}\" has a path that is not a valid pattern ({path:?}): {message}")]
    BadGlob { area: String, path: String, message: String },
}

/// Parses a configuration. Unknown top-level sections are skipped and listed in `ignored`, so a file written for a
/// newer PDT still works on an older one; unknown keys inside a `[[run]]`, `[[test]]` or `[[area]]` are refused.
pub fn parse(text: &str, origin: Source) -> Result<ConfigFile, RunConfigError> {
    let err = |e: toml::de::Error| RunConfigError::Parse { origin, message: e.to_string() };
    let mut file: ConfigFile = toml::from_str(text).map_err(err)?;
    let table: toml::Table = toml::from_str(text).map_err(err)?;
    file.forge = table.get("forge").and_then(|v| v.clone().try_into::<ForgeKind>().ok());
    let unknown_forge = table.contains_key("forge") && file.forge.is_none();
    file.ignored = table
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| !SECTIONS.contains(&k.as_str()) || (k == "forge" && unknown_forge))
        .collect();
    Ok(file)
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
                b.detach = o.detach.or(b.detach);
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
    let detach = e.detach.unwrap_or(false);
    if test && detach {
        return Err(RunConfigError::DetachedTest(name));
    }
    let setup = e.setup.filter(|s| !s.trim().is_empty());
    Ok(RunDef { name, setup, command, web, data: e.data.unwrap_or(false), detach, env })
}

/// Areas merge by name like runs: an override's fields replace the file's.
fn merge_areas(base: Vec<AreaFile>, over: Vec<AreaFile>) -> Result<Vec<AreaDef>, RunConfigError> {
    let mut out: Vec<AreaFile> = Vec::new();
    for a in base {
        if out.iter().any(|o| o.name.trim() == a.name.trim()) {
            return Err(RunConfigError::DuplicateArea(a.name.trim().into()));
        }
        out.push(a);
    }
    let mut seen_over: Vec<String> = Vec::new();
    for o in over {
        let name = o.name.trim().to_string();
        if seen_over.contains(&name) {
            return Err(RunConfigError::DuplicateArea(name));
        }
        seen_over.push(name.clone());
        match out.iter_mut().find(|b| b.name.trim() == name) {
            Some(b) => {
                b.paths = o.paths.or(b.paths.take());
                b.exclusive = o.exclusive.or(b.exclusive);
            }
            None => out.push(o),
        }
    }
    out.into_iter().map(finish_area).collect()
}

fn finish_area(a: AreaFile) -> Result<AreaDef, RunConfigError> {
    let name = crate::ticket::normalize_areas(std::slice::from_ref(&a.name))
        .ok()
        .and_then(|v| v.into_iter().next())
        .ok_or_else(|| RunConfigError::BadAreaName(a.name.clone()))?;
    let paths: Vec<String> =
        a.paths.unwrap_or_default().iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    if paths.is_empty() {
        return Err(RunConfigError::NoAreaPaths(name));
    }
    for p in &paths {
        crate::overlap::glob(p).map_err(|message| RunConfigError::BadGlob {
            area: name.clone(),
            path: p.clone(),
            message,
        })?;
    }
    Ok(AreaDef { name, paths, exclusive: a.exclusive.unwrap_or(false) })
}

/// The effective configuration: `file` with `overrides` merged over it (either may be absent).
pub fn merge(file: Option<ConfigFile>, overrides: Option<ConfigFile>) -> Result<RunConfig, RunConfigError> {
    let file = file.unwrap_or_default();
    let over = overrides.unwrap_or_default();
    let runs = merge_list(file.runs, over.runs)?.into_iter().map(|e| finish(e, false)).collect::<Result<_, _>>()?;
    let tests = merge_list(file.tests, over.tests)?.into_iter().map(|e| finish(e, true)).collect::<Result<_, _>>()?;
    let areas = merge_areas(file.areas, over.areas)?;
    let forge = over.forge.or(file.forge);
    let command = |c: Option<CiFile>| c.and_then(|c| c.command).map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    let ci = command(over.ci).or(command(file.ci)).map(|command| CiDef { command });
    let mut ignored = file.ignored;
    ignored.extend(over.ignored.into_iter().filter(|s| !ignored.contains(s)).collect::<Vec<_>>());
    Ok(RunConfig { runs, tests, areas, forge, ci, ignored })
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
            detach: false,
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
    fn detach_is_parsed_merged_and_refused_on_tests() {
        let f = parse("[[run]]\nname = \"app\"\ncommand = \"python -m app\"\ndetach = true\n", Source::File).unwrap();
        let c = merge(Some(f.clone()), None).unwrap();
        assert!(c.runs[0].detach);
        let off = parse("[[run]]\nname = \"app\"\ndetach = false\n", Source::Overrides).unwrap();
        assert!(!merge(Some(f), Some(off)).unwrap().runs[0].detach, "an override turns it off");
        assert!(!merge(Some(file()), None).unwrap().runs[0].detach, "off unless asked for");
        let t = parse("[[test]]\nname = \"unit\"\ncommand = \"x\"\ndetach = true\n", Source::File).unwrap();
        assert_eq!(merge(Some(t), None), Err(RunConfigError::DetachedTest("unit".into())));
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

    #[test]
    fn the_forge_is_named_at_the_top_and_the_overrides_win() {
        let f = parse("forge = \"github\"\n", Source::File).unwrap();
        assert_eq!((f.forge.clone(), f.ignored.is_empty()), (Some(ForgeKind::GITHUB), true));
        assert_eq!(merge(Some(f.clone()), None).unwrap().forge, Some(ForgeKind::GITHUB));
        let none = parse("forge = \"none\"\n", Source::Overrides).unwrap();
        assert_eq!(merge(Some(f), Some(none)).unwrap().forge, Some(ForgeKind::NONE));
        assert_eq!(merge(None, None).unwrap().forge, None, "chosen from the origin");
        // Any plugin's id is a forge (PDT-241): one this PDT has no plugin for is left to the origin when it is used.
        let later = parse("forge = \"gitlab\"\n[[run]]\nname = \"a\"\ncommand = \"x\"\n", Source::File).unwrap();
        assert_eq!((later.forge.clone(), later.ignored.is_empty()), (ForgeKind::new("gitlab"), true));
        assert_eq!(merge(Some(later), None).unwrap().runs.len(), 1);
        // One that can't be an id is skipped like an unknown section, so the file still works.
        let bad = parse("forge = \"Git Lab\"\n", Source::File).unwrap();
        assert_eq!((bad.forge, bad.ignored.as_slice()), (None, ["forge".to_string()].as_slice()));
    }

    #[test]
    fn the_local_forge_and_its_ci_command() {
        let f = parse("forge = \"local\"\n[ci]\ncommand = \"cargo test\"\n", Source::File).unwrap();
        assert!(f.ignored.is_empty(), "{:?}", f.ignored);
        let c = merge(Some(f.clone()), None).unwrap();
        assert_eq!(c.forge, Some(ForgeKind::LOCAL));
        assert_eq!(c.ci, Some(CiDef { command: "cargo test".into() }));
        let over = parse("[ci]\ncommand = \"make check\"\n", Source::Overrides).unwrap();
        assert_eq!(merge(Some(f.clone()), Some(over)).unwrap().ci.unwrap().command, "make check", "the overrides win");
        let blank = parse("[ci]\ncommand = \" \"\n", Source::Overrides).unwrap();
        assert_eq!(merge(Some(f), Some(blank)).unwrap().ci.unwrap().command, "cargo test", "a blank one is none");
        assert!(matches!(parse("[ci]\ncmd = \"x\"\n", Source::File), Err(RunConfigError::Parse { .. })));
    }

    #[test]
    fn unknown_sections_are_skipped_but_unknown_entry_keys_are_refused() {
        let text = "version = 2\n[[run]]\nname = \"a\"\ncommand = \"x\"\n[[pipeline]]\nname = \"p\"\n[later]\nx = 1\n";
        let f = parse(text, Source::File).unwrap();
        assert_eq!(f.ignored, ["later", "pipeline", "version"]);
        let over = parse("[[pipeline]]\nname = \"q\"\n[[gate]]\nname = \"g\"\n", Source::Overrides).unwrap();
        let c = merge(Some(f), Some(over)).unwrap();
        assert_eq!(c.runs[0].name, "a");
        assert_eq!(c.ignored, ["later", "pipeline", "version", "gate"]);
        assert!(parse("[[run]]\nname = \"a\"\ncommand = \"x\"\n", Source::File).unwrap().ignored.is_empty());

        // A typo inside an entry is still an error, and still says where it is.
        for entry in ["[[run]]\nname = \"a\"\ncommand = \"x\"\n", "[[test]]\nname = \"a\"\ncommand = \"x\"\n"] {
            let e = parse(&format!("{entry}comand = \"y\"\n[[pipeline]]\n"), Source::File).unwrap_err();
            assert!(e.to_string().contains("comand") && e.to_string().contains("line 4"), "{e}");
        }
        assert!(parse("[[area]]\nname = \"a\"\npaths = [\"x\"]\nexclusiv = true\n[x]\n", Source::File).is_err());
        // A known section of the wrong shape is still refused.
        assert!(parse("run = 1\n", Source::File).is_err());
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
    fn areas_parse_merge_and_validate() {
        let file = parse(
            r#"
[[area]]
name = " migrations "
paths = ["crates/pdt-store/migrations/**"]
exclusive = true

[[area]]
name = "web/ui"
paths = ["web/ui/**", " "]
"#,
            Source::File,
        )
        .unwrap();
        let c = merge(Some(file.clone()), None).unwrap();
        assert_eq!(
            c.areas,
            [
                AreaDef {
                    name: "migrations".into(),
                    paths: vec!["crates/pdt-store/migrations/**".into()],
                    exclusive: true
                },
                AreaDef { name: "web/ui".into(), paths: vec!["web/ui/**".into()], exclusive: false },
            ]
        );
        let over = parse(
            "[[area]]\nname = \"web/ui\"\nexclusive = true\n[[area]]\nname = \"docs\"\npaths = [\"docs/**\"]\n",
            Source::Overrides,
        )
        .unwrap();
        let c = merge(Some(file), Some(over)).unwrap();
        assert_eq!(c.areas[1], AreaDef { name: "web/ui".into(), paths: vec!["web/ui/**".into()], exclusive: true });
        assert_eq!(c.areas[2].name, "docs");

        let bad = |text: &str| merge(Some(parse(text, Source::File).unwrap()), None).unwrap_err();
        assert_eq!(bad("[[area]]\nname = \"a\"\n"), RunConfigError::NoAreaPaths("a".into()));
        assert_eq!(bad("[[area]]\nname = \"a,b\"\npaths = [\"x\"]\n"), RunConfigError::BadAreaName("a,b".into()));
        assert_eq!(bad("[[area]]\nname = \" \"\npaths = [\"x\"]\n"), RunConfigError::BadAreaName(" ".into()));
        assert_eq!(
            bad("[[area]]\nname = \"a\"\npaths = [\"x\"]\n[[area]]\nname = \"a\"\npaths = [\"y\"]\n"),
            RunConfigError::DuplicateArea("a".into())
        );
        assert!(matches!(bad("[[area]]\nname = \"a\"\npaths = [\"src/[\"]\n"), RunConfigError::BadGlob { .. }));
        assert!(parse("[[area]]\nname = \"a\"\npaths = [\"x\"]\nexclusiv = true\n", Source::File).is_err());
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
        assert!(c.ignored.is_empty());
        let area = |n: &str| c.areas.iter().find(|a| a.name == n).unwrap_or_else(|| panic!("no area {n}"));
        assert!(area("migrations").exclusive);
        assert_eq!(area("migrations").paths, ["crates/pdt-store/migrations/**"]);
        for n in ["design system", "tickets", "terminals", "runs", "workspaces", "host and updates", "ci"] {
            assert!(!area(n).exclusive, "{n}");
        }
    }
}
