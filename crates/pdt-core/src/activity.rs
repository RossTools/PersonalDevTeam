//! Agent activity (remote-host spec §5): what an agent terminal is doing, and whether the user has seen it since it
//! last needed them. Pure; pdt-api feeds it hook reports and terminal input/output.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Activity {
    Working,
    NeedsYou,
    Idle,
    Unknown,
}

impl Activity {
    pub fn as_str(self) -> &'static str {
        match self {
            Activity::Working => "working",
            Activity::NeedsYou => "needs_you",
            Activity::Idle => "idle",
            Activity::Unknown => "unknown",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [Activity::Working, Activity::NeedsYou, Activity::Idle, Activity::Unknown].into_iter().find(|a| a.as_str() == s)
    }
}

/// What `pdtd hook` posts to `/api/sessions/{id}/activity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "source", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum HookReport {
    Claude {
        event: String,
        #[serde(default)]
        #[ts(optional)]
        notification_type: Option<String>,
    },
    Codex {
        event: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Busy,
    NeedsYou,
    TurnEnded,
    /// No output for `QUIET_AFTER_MS` while working.
    Quiet,
    /// Claude took input and printed, but no hook arrived within `HOOK_GRACE_MS`: detection is broken.
    HookMissing,
}

const NEEDS_YOU_NOTIFICATIONS: [&str; 4] =
    ["permission_prompt", "elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"];

impl HookReport {
    pub fn signal(&self) -> Option<Signal> {
        match self {
            HookReport::Claude { event, notification_type } => match event.as_str() {
                "UserPromptSubmit" | "PreToolUse" => Some(Signal::Busy),
                "PermissionRequest" => Some(Signal::NeedsYou),
                "Stop" => Some(Signal::TurnEnded),
                "Notification" => match notification_type.as_deref() {
                    Some(t) if NEEDS_YOU_NOTIFICATIONS.contains(&t) => Some(Signal::NeedsYou),
                    Some("idle_prompt") => Some(Signal::TurnEnded),
                    _ => None,
                },
                _ => None,
            },
            HookReport::Codex { event } => (event == "agent-turn-complete").then_some(Signal::TurnEnded),
        }
    }
}

pub fn next(current: Activity, s: Signal) -> Activity {
    match s {
        Signal::Busy => Activity::Working,
        Signal::NeedsYou => Activity::NeedsYou,
        Signal::TurnEnded => Activity::Idle,
        Signal::Quiet if current == Activity::Working => Activity::Unknown,
        Signal::Quiet => current,
        Signal::HookMissing => Activity::Unknown,
    }
}

/// Becoming idle or needing the user (from anything else) asks for attention.
pub fn needs_attention(prev: Activity, next: Activity) -> bool {
    prev != next && matches!(next, Activity::Idle | Activity::NeedsYou)
}

pub fn is_unseen(attention_at: Option<i64>, last_viewed_at: Option<i64>) -> bool {
    attention_at.is_some_and(|a| last_viewed_at.is_none_or(|v| a > v))
}

pub const QUIET_AFTER_MS: i64 = 120_000;
pub const HOOK_GRACE_MS: i64 = 20_000;

/// Per-session timing facts the heuristics need.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Watch {
    pub last_output_ms: i64,
    /// When the user last pressed Enter (Claude only), until a hook arrives.
    pub enter_at_ms: Option<i64>,
    pub hook_since_enter: bool,
    pub output_since_enter: bool,
}

/// The time-based signal due now, if any. `expects_hooks` is true for Claude.
pub fn tick(activity: Activity, w: &Watch, expects_hooks: bool, now: i64) -> Option<Signal> {
    if expects_hooks
        && let Some(at) = w.enter_at_ms
        && !w.hook_since_enter
        && w.output_since_enter
        && now - at >= HOOK_GRACE_MS
    {
        return Some(Signal::HookMissing);
    }
    (activity == Activity::Working && now - w.last_output_ms >= QUIET_AFTER_MS).then_some(Signal::Quiet)
}

/// Lines of Codex's approval prompt, as verified in Task 1 (docs/superpowers/notes/phase-2-agent-signals.md).
/// Compared after `strip_ansi` + `squash`, because the TUI redraws with cursor moves instead of spaces, and ConPTY
/// skips characters already on screen ("Would you like to run" arrives as "Wou…d"), so only verified tails are used.
pub const CODEX_APPROVAL_MARKERS: &[&str] = &[
    "to run the following command?",
    "Yes, proceed",
    "No, and tell Codex what to do differently",
    "Press enter to confirm or esc to cancel",
    // Secondary: the edit-approval question. Not verified on a real screen in Task 1.
    "to make the following edits?",
];

/// Terminal output without escape sequences (CSI, OSC, two-byte ESC) or control characters other than newlines.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x1b => {
                i += 1;
                match bytes.get(i) {
                    Some(b'[') => {
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                        i += 1;
                    }
                    Some(b']') => {
                        i += 1;
                        while i < bytes.len()
                            && bytes[i] != 0x07
                            && !(bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\'))
                        {
                            i += 1;
                        }
                        i += if bytes.get(i) == Some(&0x1b) { 2 } else { 1 };
                    }
                    Some(_) => i += 1,
                    None => {}
                }
            }
            b'\n' => {
                out.push(b'\n');
                i += 1;
            }
            c if c < 0x20 || c == 0x7f => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lower-case, without whitespace.
pub fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// Whether squashed screen text contains any marker.
pub fn screen_shows(squashed: &str, markers: &[&str]) -> bool {
    markers.iter().any(|m| squashed.contains(&squash(m)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Activity::*;

    fn claude(event: &str, nt: Option<&str>) -> HookReport {
        HookReport::Claude { event: event.into(), notification_type: nt.map(Into::into) }
    }

    #[test]
    fn wire_format() {
        let r: HookReport = serde_json::from_str(
            r#"{"source":"claude","event":"Notification","notificationType":"permission_prompt"}"#,
        )
        .unwrap();
        assert_eq!(r, claude("Notification", Some("permission_prompt")));
        let r: HookReport = serde_json::from_str(r#"{"source":"codex","event":"agent-turn-complete"}"#).unwrap();
        assert_eq!(r, HookReport::Codex { event: "agent-turn-complete".into() });
        assert_eq!(serde_json::to_string(&NeedsYou).unwrap(), r#""needs_you""#);
        for a in [Working, NeedsYou, Idle, Unknown] {
            assert_eq!(Activity::parse(a.as_str()), Some(a));
        }
    }

    #[test]
    fn claude_events_map_to_signals() {
        assert_eq!(claude("UserPromptSubmit", None).signal(), Some(Signal::Busy));
        assert_eq!(claude("PreToolUse", None).signal(), Some(Signal::Busy));
        assert_eq!(claude("PermissionRequest", None).signal(), Some(Signal::NeedsYou));
        for nt in ["permission_prompt", "elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"] {
            assert_eq!(claude("Notification", Some(nt)).signal(), Some(Signal::NeedsYou), "{nt}");
        }
        assert_eq!(claude("Notification", Some("idle_prompt")).signal(), Some(Signal::TurnEnded));
        assert_eq!(claude("Notification", Some("auth_success")).signal(), None);
        assert_eq!(claude("Notification", None).signal(), None);
        assert_eq!(claude("Stop", None).signal(), Some(Signal::TurnEnded));
        assert_eq!(claude("SessionStart", None).signal(), None);
        assert_eq!(HookReport::Codex { event: "agent-turn-complete".into() }.signal(), Some(Signal::TurnEnded));
        assert_eq!(HookReport::Codex { event: "something-new".into() }.signal(), None);
    }

    #[test]
    fn transitions() {
        assert_eq!(next(Idle, Signal::Busy), Working);
        assert_eq!(next(Working, Signal::NeedsYou), NeedsYou);
        assert_eq!(next(NeedsYou, Signal::Busy), Working);
        assert_eq!(next(Working, Signal::TurnEnded), Idle);
        assert_eq!(next(Working, Signal::Quiet), Unknown);
        assert_eq!(next(Idle, Signal::Quiet), Idle, "quiet only matters while working");
        assert_eq!(next(Idle, Signal::HookMissing), Unknown, "hooks broke: say so");
        assert_eq!(next(Unknown, Signal::TurnEnded), Idle);
    }

    #[test]
    fn attention_and_unseen() {
        assert!(needs_attention(Working, Idle));
        assert!(needs_attention(Unknown, Idle));
        assert!(needs_attention(Idle, NeedsYou));
        assert!(!needs_attention(Idle, Idle));
        assert!(!needs_attention(Idle, Working));
        assert!(!needs_attention(NeedsYou, NeedsYou));
        assert!(!is_unseen(None, None));
        assert!(is_unseen(Some(10), None));
        assert!(is_unseen(Some(10), Some(5)));
        assert!(!is_unseen(Some(10), Some(10)), "viewed at the same moment counts as seen");
        assert!(!is_unseen(Some(10), Some(11)));
    }

    #[test]
    fn ticks() {
        let w = |last_output_ms, enter_at_ms, hook_since_enter, output_since_enter| Watch {
            last_output_ms,
            enter_at_ms,
            hook_since_enter,
            output_since_enter,
        };
        assert_eq!(tick(Working, &w(0, None, false, false), true, QUIET_AFTER_MS), Some(Signal::Quiet));
        assert_eq!(tick(Working, &w(1, None, false, false), true, QUIET_AFTER_MS), None);
        assert_eq!(tick(Idle, &w(0, None, false, false), true, QUIET_AFTER_MS * 10), None);
        let missing = w(HOOK_GRACE_MS, Some(0), false, true);
        assert_eq!(tick(Idle, &missing, true, HOOK_GRACE_MS), Some(Signal::HookMissing));
        assert_eq!(tick(Idle, &missing, false, HOOK_GRACE_MS), None, "Codex has no per-prompt hook");
        assert_eq!(tick(Idle, &w(HOOK_GRACE_MS, Some(0), true, true), true, HOOK_GRACE_MS), None, "a hook came");
        assert_eq!(tick(Idle, &w(0, Some(0), false, false), true, HOOK_GRACE_MS), None, "no output: nothing happened");
    }

    #[test]
    fn screen_text() {
        let raw =
            b"\x1b[2J\x1b[1;1H\x1b[1mWould\x1b[0m you\x1b[3C like to \x1b]0;title\x07run the following command?\r\n";
        let text = strip_ansi(raw);
        assert!(!text.contains('\x1b'));
        assert!(!text.contains("title"), "OSC payloads are dropped");
        // Task 1: ConPTY drops characters at the start of the question, so only its tail is a verified marker.
        assert!(screen_shows(&squash(&text), &["to run the following command?"]));
        assert!(screen_shows(&squash(&text), CODEX_APPROVAL_MARKERS));
        assert!(!screen_shows(&squash("nothing here"), CODEX_APPROVAL_MARKERS));
        assert_eq!(squash(" A b\tC\n"), "abc");
    }
}
