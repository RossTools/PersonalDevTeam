//! Per-session agent hook configuration (remote-host spec §5.1, decisions 11–12). The user's own settings are never
//! touched: Claude gets `--settings <file>`, Codex gets `-c notify=[…]`.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const CLAUDE_HOOK_EVENTS: [&str; 5] =
    ["UserPromptSubmit", "PreToolUse", "PermissionRequest", "Notification", "Stop"];
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
}
