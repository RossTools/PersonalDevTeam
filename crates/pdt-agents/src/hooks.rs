//! Per-session agent hook configuration. The user's own settings are never
//! touched: Claude gets `--settings <file>`, Codex gets `-c notify=[…]`, Devin gets `--config <file>` (a copy of the
//! user's Devin config with PDT's hook added, since `--config` replaces the user's file).

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// SessionStart changes nothing by itself; it shows early that the session's hooks work (see `Watch::hook_seen`).
pub const CLAUDE_HOOK_EVENTS: [&str; 7] =
    ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Notification", "Stop"];
pub const CLAUDE_SETTINGS_FILE: &str = "claude-settings.json";

/// Exec-form command hooks (no shell, verified in Task 1) running `<pdtd> hook claude <Event>`.
pub fn claude_settings(pdtd: &Path) -> Value {
    let mut hooks = serde_json::Map::new();
    for ev in CLAUDE_HOOK_EVENTS {
        hooks.insert(
            ev.into(),
            json!([{ "hooks": [{
                "type": "command",
                "command": pdtd.to_string_lossy(),
                "args": ["hook", "claude", ev],
                "timeout": 10
            }] }]),
        );
    }
    json!({ "hooks": hooks })
}

/// Writes `<dir>/claude-settings.json` and returns its path.
pub fn write_claude_settings(dir: &Path, pdtd: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(CLAUDE_SETTINGS_FILE);
    std::fs::write(&path, serde_json::to_vec_pretty(&claude_settings(pdtd)).expect("json"))?;
    Ok(path)
}

/// A TOML basic string.
pub fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `-c notify=[<pdtd>, "hook", "codex-notify", ("--chain", <user's notify…>, "--")]`; Codex appends its JSON.
pub fn codex_notify_args(pdtd: &Path, chain: &[String]) -> Vec<String> {
    let mut items: Vec<String> = vec![pdtd.to_string_lossy().into_owned(), "hook".into(), "codex-notify".into()];
    if !chain.is_empty() {
        items.push("--chain".into());
        items.extend(chain.iter().cloned());
        items.push("--".into());
    }
    let list = items.iter().map(|i| toml_string(i)).collect::<Vec<_>>().join(",");
    vec!["-c".into(), format!("notify=[{list}]")]
}

/// The user's own top-level `notify` from `<codex_home>/config.toml`, to chain to (never our own hook).
pub fn user_codex_notify(codex_home: &Path, pdtd: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(codex_home.join("config.toml")).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    let items: Vec<String> =
        table.get("notify")?.as_array()?.iter().map(|v| v.as_str().map(String::from)).collect::<Option<_>>()?;
    let ours = items.len() > 1 && items[1] == "hook" && Path::new(&items[0]) == pdtd;
    (!items.is_empty() && !ours).then_some(items)
}

pub const DEVIN_CONFIG_FILE: &str = "devin-config.json";

/// The user's own Devin config file: `%APPDATA%\devin\config.json` on Windows, else
/// `$XDG_CONFIG_HOME/devin/config.json` (default `~/.config`).
pub fn devin_user_config_file(
    windows: bool,
    appdata: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let base = if windows {
        appdata?
    } else {
        xdg_config_home.filter(|p| p.is_absolute()).or_else(|| home.map(|h| h.join(".config")))?
    };
    Some(base.join("devin").join("config.json"))
}

/// One word of a hook command: Devin runs hook commands through a shell and ignores an `args` list (verified with
/// Devin 3000.11), so PDT quotes the program itself. POSIX single quotes; on Windows double quotes.
fn shell_word(s: &str, windows: bool) -> String {
    if windows { format!("\"{s}\"") } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// The hook command PDT gives a Devin session: `<pdtd> hook devin`, plus `--instructions <file>` (printed back to
/// Devin as `SessionStart` context) when there are instructions.
pub fn devin_hook_command(pdtd: &Path, instructions: Option<&Path>, windows: bool) -> String {
    let mut cmd = format!("{} hook devin", shell_word(&pdtd.to_string_lossy(), windows));
    if let Some(file) = instructions {
        cmd.push_str(" --instructions ");
        cmd.push_str(&shell_word(&file.to_string_lossy(), windows));
    }
    cmd
}

/// The config a Devin session gets: the user's own config, with PDT's `SessionStart` hook added after any of the
/// user's own `SessionStart` hooks. Devin reports activity itself (OSC 7501), so that one hook is all PDT needs: it
/// names the conversation (for resume) and hands Devin PDT's instructions.
pub fn devin_config(user: Option<Value>, hook_command: &str) -> Value {
    let mut config = match user {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    let ours = json!({ "matcher": "", "hooks": [{ "type": "command", "command": hook_command, "timeout": 10 }] });
    let hooks = config.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let start = hooks.as_object_mut().expect("object").entry("SessionStart").or_insert_with(|| json!([]));
    match start {
        Value::Array(list) => list.push(ours),
        other => *other = json!([ours]),
    }
    Value::Object(config)
}

/// Writes `<dir>/devin-config.json` from the user's config file (`user_file`; a missing file is an empty config) and
/// returns its path. Ok(None) when the user's file cannot be read or parsed: the session then runs on the user's
/// own config without PDT's hook, rather than on a copy that lost their settings.
pub fn write_devin_config(
    dir: &Path,
    user_file: Option<&Path>,
    hook_command: &str,
) -> std::io::Result<Option<PathBuf>> {
    let user = match user_file.map(std::fs::read_to_string) {
        None => None,
        Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Some(Err(_)) => return Ok(None),
        Some(Ok(text)) => match parse_jsonc(&text) {
            Some(v) if v.is_object() => Some(v),
            _ => return Ok(None),
        },
    };
    std::fs::create_dir_all(dir)?;
    let path = dir.join(DEVIN_CONFIG_FILE);
    std::fs::write(&path, serde_json::to_vec_pretty(&devin_config(user, hook_command)).expect("json"))?;
    Ok(Some(path))
}

/// JSON with comments (`//`, `/* */`) and trailing commas, as Devin's config files allow.
pub fn parse_jsonc(text: &str) -> Option<Value> {
    serde_json::from_str(&strip_jsonc(text)).ok()
}

fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        out.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    if chars[i - 1] == '"' {
                        break;
                    }
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
            }
            ',' => {
                // A trailing comma: the next significant character closes the object or array.
                if !matches!(next_significant(&chars, i + 1), Some('}') | Some(']')) {
                    out.push(c);
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The first character from `j` on that is not whitespace or inside a comment. A linear scan: re-stripping the rest
/// of the text at every `, // …` would double the work with each one.
fn next_significant(chars: &[char], mut j: usize) -> Option<char> {
    loop {
        match chars.get(j) {
            Some(w) if w.is_whitespace() => j += 1,
            Some('/') if chars.get(j + 1) == Some(&'/') => {
                while j < chars.len() && chars[j] != '\n' {
                    j += 1;
                }
            }
            Some('/') if chars.get(j + 1) == Some(&'*') => {
                j += 2;
                while j < chars.len() && !(chars[j] == '*' && chars.get(j + 1) == Some(&'/')) {
                    j += 1;
                }
                j += 2;
            }
            other => return other.copied(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_settings_use_exec_form_for_every_event() {
        let exe = Path::new(if cfg!(windows) {
            r"C:\Program Files\P D T\pdtd.exe"
        } else {
            "/Applications/P D T.app/Contents/MacOS/pdtd"
        });
        let v = claude_settings(exe);
        for ev in ["SessionStart", "PostToolUse", "PreToolUse", "Stop"] {
            assert!(CLAUDE_HOOK_EVENTS.contains(&ev), "{ev} is subscribed");
        }
        for ev in CLAUDE_HOOK_EVENTS {
            let h = &v["hooks"][ev][0]["hooks"][0];
            assert_eq!(h["type"], "command");
            assert_eq!(h["command"], exe.to_string_lossy().as_ref());
            assert_eq!(h["args"], json!(["hook", "claude", ev]));
            assert!(v["hooks"][ev][0].get("matcher").is_none(), "no matcher: every tool and notification");
        }
        let dir = tempfile::tempdir().unwrap();
        let p = write_claude_settings(&dir.path().join("s1"), exe).unwrap();
        let back: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn notify_argument_is_valid_toml_with_windows_paths() {
        let args = codex_notify_args(Path::new(r#"C:\Users\o"neil\pdtd.exe"#), &[]);
        assert_eq!(args[0], "-c");
        let parsed: toml::Table = toml::from_str(&args[1]).unwrap();
        let list: Vec<&str> = parsed["notify"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(list, [r#"C:\Users\o"neil\pdtd.exe"#, "hook", "codex-notify"]);
        let chained = codex_notify_args(Path::new("/x/pdtd"), &["notifier".into(), "turn-ended".into()]);
        let parsed: toml::Table = toml::from_str(&chained[1]).unwrap();
        let list: Vec<&str> = parsed["notify"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(list, ["/x/pdtd", "hook", "codex-notify", "--chain", "notifier", "turn-ended", "--"]);
    }

    #[test]
    fn toml_strings_escape_control_characters() {
        let s = "a\tb\nc\u{1}d\u{7f}\\\"";
        let parsed: toml::Table = toml::from_str(&format!("v = {}", toml_string(s))).unwrap();
        assert_eq!(parsed["v"].as_str(), Some(s));
    }

    #[test]
    fn reads_the_users_notify_but_never_our_own() {
        let dir = tempfile::tempdir().unwrap();
        let pdtd = Path::new("/x/pdtd");
        assert_eq!(user_codex_notify(dir.path(), pdtd), None, "no config");
        std::fs::write(
            dir.path().join("config.toml"),
            "model = \"o3\"\nnotify = [\"C:\\\\n.exe\", \"turn-ended\"]\n[tui]\n",
        )
        .unwrap();
        assert_eq!(user_codex_notify(dir.path(), pdtd), Some(vec![r"C:\n.exe".to_string(), "turn-ended".into()]));
        std::fs::write(dir.path().join("config.toml"), "notify = [\"/x/pdtd\", \"hook\", \"codex-notify\"]\n").unwrap();
        assert_eq!(user_codex_notify(dir.path(), pdtd), None);
        std::fs::write(dir.path().join("config.toml"), "notify = \"oops\"\n").unwrap();
        assert_eq!(user_codex_notify(dir.path(), pdtd), None);
        std::fs::write(dir.path().join("config.toml"), "notify = []\n").unwrap();
        assert_eq!(user_codex_notify(dir.path(), pdtd), None);
    }
    #[test]
    fn devin_config_keeps_the_users_settings_and_hooks() {
        let user = parse_jsonc(
            r#"{
              // the user's own
              "version": 1, "devin": { "org_id": "org-1" },
              "theme_mode": "dark", /* block */ "url": "http://x//y",
              "hooks": { "SessionStart": [{ "matcher": "", "hooks": [{ "type": "command", "command": "mine" }] }],
                         "Stop": [], },
            }"#,
        )
        .expect("JSONC parses");
        let cmd = devin_hook_command(Path::new("/A p/pdtd"), Some(Path::new("/d/it's.md")), false);
        assert_eq!(cmd, r#"'/A p/pdtd' hook devin --instructions '/d/it'\''s.md'"#);
        let v = devin_config(Some(user), &cmd);
        assert_eq!(v["devin"]["org_id"], "org-1");
        assert_eq!(v["theme_mode"], "dark");
        assert_eq!(v["url"], "http://x//y", "comment markers inside strings are kept");
        assert_eq!(v["hooks"]["Stop"], json!([]));
        let start = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 2, "the user's hook stays first");
        assert_eq!(start[0]["hooks"][0]["command"], "mine");
        assert_eq!(start[1]["hooks"][0]["command"], cmd);
        assert_eq!(start[1]["hooks"][0]["type"], "command");
        let fresh = devin_config(None, "x");
        assert_eq!(fresh["hooks"]["SessionStart"][0]["hooks"][0]["command"], "x");
        assert_eq!(
            devin_hook_command(Path::new(r"C:\P D T\pdtd.exe"), None, true),
            r#""C:\P D T\pdtd.exe" hook devin"#
        );
    }

    #[test]
    fn jsonc_with_many_commented_commas_parses_in_linear_time() {
        let mut text = String::from("{\n");
        for n in 0..200 {
            text.push_str(&format!("  \"k{n}\": {n}, // note {n}\n"));
        }
        text.push_str("  \"last\": [1, /* a */ 2, // b\n /* c */ ], // trailing\n}");
        let started = std::time::Instant::now();
        let v = parse_jsonc(&text).expect("JSONC parses");
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "took {:?}", started.elapsed());
        assert_eq!(v["k0"], 0);
        assert_eq!(v["k199"], 199);
        assert_eq!(v["last"], json!([1, 2]));
        assert_eq!(parse_jsonc("[1, /* unclosed"), None);
    }

    #[test]
    fn devin_config_file_is_written_only_from_a_readable_user_config() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("config.json");
        let out = dir.path().join("s1");
        // No user config yet: just PDT's hook.
        let p = write_devin_config(&out, Some(&user), "h").unwrap().unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["SessionStart"][0]["hooks"][0]["command"], "h");
        std::fs::write(&user, "{ \"theme_mode\": \"light\", }").unwrap();
        let v: Value = serde_json::from_slice(
            &std::fs::read(write_devin_config(&out, Some(&user), "h").unwrap().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(v["theme_mode"], "light");
        // A config PDT cannot read is never replaced by a copy that lost it.
        std::fs::write(&user, "{ not json").unwrap();
        assert_eq!(write_devin_config(&out, Some(&user), "h").unwrap(), None);
        std::fs::write(&user, "[1]").unwrap();
        assert_eq!(write_devin_config(&out, Some(&user), "h").unwrap(), None);
    }

    #[test]
    fn devin_user_config_location() {
        // Absolute on every OS ("/x" is not absolute on Windows).
        let home = std::env::temp_dir().join("u");
        let xdg = std::env::temp_dir().join("x");
        let default = home.join(".config").join("devin").join("config.json");
        assert_eq!(devin_user_config_file(false, None, None, Some(home.clone())), Some(default.clone()));
        assert_eq!(
            devin_user_config_file(false, None, Some(xdg.clone()), Some(home.clone())),
            Some(xdg.join("devin").join("config.json"))
        );
        assert_eq!(
            devin_user_config_file(false, None, Some(PathBuf::from("rel")), Some(home.clone())),
            Some(default),
            "a relative XDG_CONFIG_HOME is ignored"
        );
        let home = Some(home);
        let appdata = PathBuf::from(r"C:\Users\me\AppData\Roaming");
        assert_eq!(
            devin_user_config_file(true, Some(appdata.clone()), None, home),
            Some(appdata.join("devin").join("config.json"))
        );
        assert_eq!(devin_user_config_file(true, None, None, None), None);
    }
}
