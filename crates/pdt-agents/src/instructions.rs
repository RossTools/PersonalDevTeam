//! How PDT's instructions for a session (pdt-core `agent_context`) reach each agent without touching the user's own
//! settings: Claude appends a file to its system prompt; Codex gets them as `developer_instructions`, after the
//! user's own `developer_instructions` from config.toml (which PDT's value would otherwise replace).

use crate::hooks::toml_string;
use std::path::{Path, PathBuf};

pub const INSTRUCTIONS_FILE: &str = "instructions.md";

/// Writes `<dir>/instructions.md` and returns its path.
pub fn write_instructions(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(INSTRUCTIONS_FILE);
    std::fs::write(&path, text)?;
    Ok(path)
}

/// Claude, terminal or `-p`: `--append-system-prompt-file <file>`.
pub fn claude_args(file: &Path) -> Vec<String> {
    vec!["--append-system-prompt-file".into(), file.to_string_lossy().into_owned()]
}

/// Codex's TUI: `-c developer_instructions=<text>` as a TOML string. Through `cmd.exe` (a Windows `.cmd` shim) the
/// text itself would be mangled, so the agent is pointed at `file` instead, in a TOML literal string; nothing when
/// that path is unsafe there too.
pub fn codex_tui_args(text: &str, via_cmd: bool, file: &Path) -> Vec<String> {
    if !via_cmd {
        return vec!["-c".into(), format!("developer_instructions={}", toml_string(text))];
    }
    let path = file.to_string_lossy();
    if !pdt_core::session::cmd_safe(&path) || path.contains('\'') {
        return vec![];
    }
    vec!["-c".into(), format!("developer_instructions='Read {path} first and follow it.'")]
}

/// The user's own top-level `developer_instructions` from `<codex_home>/config.toml`.
pub fn user_codex_developer_instructions(codex_home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(codex_home.join("config.toml")).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    let v = table.get("developer_instructions")?.as_str()?.trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// The user's own instructions (kept first), then PDT's.
pub fn combine(user: Option<&str>, ours: &str) -> String {
    match user {
        Some(u) => format!("{u}\n\n{ours}"),
        None => ours.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_gets_the_text_as_a_toml_string() {
        let text = "# PDT\nUse \"make\" & `cargo`.\n";
        let a = codex_tui_args(text, false, Path::new("/d/instructions.md"));
        assert_eq!(a[0], "-c");
        let t: toml::Table = toml::from_str(&a[1]).unwrap();
        assert_eq!(t["developer_instructions"].as_str(), Some(text));
    }

    #[test]
    fn codex_through_cmd_is_pointed_at_the_file() {
        let file = Path::new(r"C:\Users\me\AppData\Local\pdt\sessions\s1\instructions.md");
        let a = codex_tui_args("x & y", true, file);
        let t: toml::Table = toml::from_str(&a[1]).unwrap();
        let v = t["developer_instructions"].as_str().unwrap();
        assert!(v.contains(&*file.to_string_lossy()) && !v.contains("x & y"), "{v}");
        assert!(!a[1].contains(['&', '"', '%', '\n']), "nothing cmd.exe would interpret: {}", a[1]);
        assert!(codex_tui_args("x", true, Path::new(r"C:\a&b\i.md")).is_empty());
        assert!(codex_tui_args("x", true, Path::new(r"C:\o'neil\i.md")).is_empty());
    }

    #[test]
    fn the_users_own_codex_instructions_come_first() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(user_codex_developer_instructions(home.path()), None);
        std::fs::write(home.path().join("config.toml"), "model = \"x\"\ndeveloper_instructions = \"Be brief.\"\n")
            .unwrap();
        let user = user_codex_developer_instructions(home.path());
        assert_eq!(user.as_deref(), Some("Be brief."));
        assert_eq!(combine(user.as_deref(), "PDT"), "Be brief.\n\nPDT");
        assert_eq!(combine(None, "PDT"), "PDT");
    }

    #[test]
    fn claude_appends_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = write_instructions(&dir.path().join("s1"), "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "hello");
        assert_eq!(claude_args(&f), ["--append-system-prompt-file".to_string(), f.to_string_lossy().into_owned()]);
    }
}
