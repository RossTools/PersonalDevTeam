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
        /// Claude's id for the conversation (it changes after `/clear` or the TUI's `/resume`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        session_id: Option<String>,
        /// SessionStart's `source`: `startup`, `resume`, `clear` or `compact`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        start_source: Option<String>,
    },
    Codex {
        event: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Explicit Codex OSC 7501 status, authoritative over inferred activity and delayed notifications.
    ProgramStatus(Activity),
    Busy,
    NeedsYou,
    TurnEnded,
    /// The TUI acknowledged a requested interruption (no Stop hook is sent).
    Interrupted,
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
            HookReport::Claude { event, notification_type, .. } => match event.as_str() {
                "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(Signal::Busy),
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
        Signal::ProgramStatus(activity) => activity,
        Signal::Busy => Activity::Working,
        Signal::NeedsYou => Activity::NeedsYou,
        Signal::TurnEnded | Signal::Interrupted => Activity::Idle,
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

/// The thresholds `tick` uses (tests shorten them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub quiet_after_ms: i64,
    pub hook_grace_ms: i64,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { quiet_after_ms: QUIET_AFTER_MS, hook_grace_ms: HOOK_GRACE_MS }
    }
}

/// Per-session timing facts the heuristics need.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Watch {
    pub last_output_ms: i64,
    /// When the user last pressed Enter (Claude only, while no hook has arrived yet).
    pub enter_at_ms: Option<i64>,
    /// Whether any hook has arrived since this terminal started: once one has, hooks work, and an Enter that sends
    /// none (an empty line, a slash command) is not a sign that they broke.
    pub hook_seen: bool,
    pub output_since_enter: bool,
}

/// The time-based signal due now, if any. `expects_hooks` is true for Claude.
pub fn tick(activity: Activity, w: &Watch, expects_hooks: bool, now: i64) -> Option<Signal> {
    tick_with(activity, w, expects_hooks, now, &Timing::default())
}

/// `tick` with the given thresholds.
pub fn tick_with(activity: Activity, w: &Watch, expects_hooks: bool, now: i64, t: &Timing) -> Option<Signal> {
    if expects_hooks
        && !w.hook_seen
        && let Some(at) = w.enter_at_ms
        && w.output_since_enter
        && now - at >= t.hook_grace_ms
    {
        return Some(Signal::HookMissing);
    }
    (activity == Activity::Working && now - w.last_output_ms >= t.quiet_after_ms).then_some(Signal::Quiet)
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

/// `CODEX_APPROVAL_MARKERS`, squashed once.
static SQUASHED_CODEX_MARKERS: std::sync::LazyLock<Vec<String>> =
    std::sync::LazyLock::new(|| CODEX_APPROVAL_MARKERS.iter().map(|m| squash(m)).collect());

/// How many distinct Codex approval markers squashed screen text contains.
pub fn codex_markers_shown(squashed: &str) -> usize {
    SQUASHED_CODEX_MARKERS.iter().filter(|m| squashed.contains(m.as_str())).count()
}

/// While Codex waits for an approval its window title says this (Task 1: `[ ! ] Action Required | <thread> | …`).
pub const CODEX_ACTION_TITLE: &str = "Action Required";

/// Whether Codex is showing its approval prompt: its window title says so, or the screen shows at least two distinct
/// prompt lines (one alone could be ordinary output).
pub fn codex_approval_shown(title_says_action: bool, squashed: &str) -> bool {
    title_says_action || codex_markers_shown(squashed) >= 2
}

/// Whether any of these window titles is Codex's "Action Required".
pub fn titles_say_action_required(titles: &[String]) -> bool {
    titles.iter().any(|t| t.contains(CODEX_ACTION_TITLE))
}

/// Window titles (OSC 0 and 2) set by complete sequences in `bytes`, and the unfinished sequence at its end (empty
/// if none), to be prepended to the next chunk.
pub fn osc_titles(bytes: &[u8]) -> (Vec<String>, &[u8]) {
    let (messages, rest) = osc_messages(bytes);
    (
        messages
            .into_iter()
            .filter_map(|body| {
                let ("0" | "2", title) = body.split_once(';')? else { return None };
                Some(title.to_string())
            })
            .collect(),
        rest,
    )
}

/// The latest Codex OSC 7501 status in this chunk, plus an unfinished OSC sequence to carry forward.
/// Codex reports idle/working/blocked/done/error; done and error both leave the TUI awaiting input.
pub fn codex_program_status(bytes: &[u8]) -> (Option<Activity>, &[u8]) {
    let (messages, rest) = osc_messages(bytes);
    let latest = messages
        .iter()
        .filter_map(|body| {
            let fields: Vec<_> = body.strip_prefix("7501;")?.split(':').collect();
            if !fields.contains(&"app=codex") {
                return None;
            }
            match fields.iter().find_map(|f| f.strip_prefix("state="))? {
                "idle" | "done" | "error" => Some(Activity::Idle),
                "working" => Some(Activity::Working),
                "blocked" => Some(Activity::NeedsYou),
                _ => None,
            }
        })
        .next_back();
    (latest, rest)
}

fn osc_messages(bytes: &[u8]) -> (Vec<String>, &[u8]) {
    let mut titles = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 == bytes.len() {
            return (titles, &bytes[i..]);
        }
        if !(bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b']')) {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 2;
        let end = loop {
            match bytes.get(j) {
                None => return (titles, &bytes[start..]),
                Some(0x07) => break (j, j + 1),
                Some(0x1b) if bytes.get(j + 1) == Some(&b'\\') => break (j, j + 2),
                Some(0x1b) if j + 1 == bytes.len() => return (titles, &bytes[start..]),
                Some(_) => j += 1,
            }
        };
        let body = String::from_utf8_lossy(&bytes[start + 2..end.0]);
        titles.push(body.into_owned());
        i = end.1;
    }
    (titles, &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use Activity::*;

    fn claude(event: &str, nt: Option<&str>) -> HookReport {
        HookReport::Claude {
            event: event.into(),
            notification_type: nt.map(Into::into),
            session_id: None,
            start_source: None,
        }
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
        assert_eq!(claude("PostToolUse", None).signal(), Some(Signal::Busy));
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
        for state in [Working, NeedsYou, Unknown] {
            assert_eq!(next(state, Signal::Interrupted), Idle);
        }
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

    fn w(last_output_ms: i64, enter_at_ms: Option<i64>, hook_seen: bool, output_since_enter: bool) -> Watch {
        Watch { last_output_ms, enter_at_ms, hook_seen, output_since_enter }
    }

    #[test]
    fn ticks() {
        assert_eq!(tick(Working, &w(0, None, false, false), true, QUIET_AFTER_MS), Some(Signal::Quiet));
        assert_eq!(tick(Working, &w(1, None, false, false), true, QUIET_AFTER_MS), None);
        assert_eq!(tick(Idle, &w(0, None, false, false), true, QUIET_AFTER_MS * 10), None);
        let missing = w(HOOK_GRACE_MS, Some(0), false, true);
        assert_eq!(tick(Idle, &missing, true, HOOK_GRACE_MS), Some(Signal::HookMissing), "hooks never fired");
        assert_eq!(tick(Idle, &missing, true, HOOK_GRACE_MS - 1), None, "still within the grace period");
        assert_eq!(tick(Idle, &missing, false, HOOK_GRACE_MS), None, "Codex has no per-prompt hook");
        assert_eq!(tick(Idle, &w(0, Some(0), false, false), true, HOOK_GRACE_MS), None, "no output: nothing happened");
        let timing = Timing { quiet_after_ms: 10, hook_grace_ms: 5 };
        assert_eq!(tick_with(Working, &w(0, None, true, false), true, 10, &timing), Some(Signal::Quiet));
        assert_eq!(tick_with(Idle, &w(0, Some(0), false, true), true, 5, &timing), Some(Signal::HookMissing));
    }

    #[test]
    fn working_hooks_mean_enter_never_makes_a_session_unknown() {
        // An empty Enter or a slash command prints but sends no hook; hooks have fired before, so they work.
        let enter_without_hook = w(HOOK_GRACE_MS * 10, Some(0), true, true);
        for a in [Idle, Working, NeedsYou] {
            assert_eq!(tick(a, &enter_without_hook, true, HOOK_GRACE_MS * 10), None, "{a:?}");
        }
        // Approving a permission prompt (a keypress) resumes the turn; a long command that keeps printing stays
        // working.
        let after_answer = next(NeedsYou, Signal::Busy);
        assert_eq!(after_answer, Working);
        let busy = w(QUIET_AFTER_MS * 3, Some(QUIET_AFTER_MS), true, true);
        assert_eq!(tick(after_answer, &busy, true, QUIET_AFTER_MS * 3 + 1_000), None);
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

    #[test]
    fn codex_approval_needs_the_title_or_two_markers() {
        let one = squash("The docs say: Yes, proceed (y) when asked.");
        assert_eq!(codex_markers_shown(&one), 1);
        assert!(!codex_approval_shown(false, &one), "a single marker in ordinary output is not the prompt");
        let same_twice = squash("Yes, proceed. Yes, proceed.");
        assert!(!codex_approval_shown(false, &same_twice), "markers must be distinct");
        let two = squash("to run the following command? ... 1. Yes, proceed (y)");
        assert!(codex_approval_shown(false, &two));
        assert!(codex_approval_shown(true, ""), "the Action Required title alone is enough");
    }

    #[test]
    fn codex_program_status_is_explicit_and_app_specific() {
        for (name, activity) in
            [("idle", Idle), ("working", Working), ("blocked", NeedsYou), ("done", Idle), ("error", Idle)]
        {
            for ending in ["\x07", "\x1b\\"] {
                let bytes = format!("\x1b]7501;state={name}:app=codex:kind=permission{ending}");
                assert_eq!(codex_program_status(bytes.as_bytes()), (Some(activity), &b""[..]));
            }
        }
        for bytes in [
            &b"state=working:app=codex"[..],
            b"\x1b]7501;state=working:app=shell\x07",
            b"\x1b]7501;state=new:app=codex\x07",
            b"\x1b]7501;state=clear\x07",
        ] {
            assert_eq!(codex_program_status(bytes).0, None);
        }
        assert_eq!(
            codex_program_status(b"\x1b]7501;state=working:app=codex\x07\x1b]7501;state=idle:app=codex\x07").0,
            Some(Idle)
        );
        let bytes = b"\x1b]7501;state=blocked:app=codex\x1b\\";
        let mut carry = Vec::new();
        let mut found = None;
        for byte in bytes {
            carry.push(*byte);
            let (activity, rest) = codex_program_status(&carry);
            found = activity.or(found);
            carry = rest.to_vec();
        }
        assert_eq!(found, Some(NeedsYou), "every byte can arrive separately");
        assert!(carry.is_empty());
    }

    #[test]
    fn window_titles_are_read_from_complete_osc_sequences() {
        let (titles, rest) =
            osc_titles(b"a\x1b]0;[ ! ] Action Required | t | f\x07b\x1b]2;two\x1b\\c\x1b]8;;http://x\x07");
        assert_eq!(titles, ["[ ! ] Action Required | t | f", "two"], "OSC 0 and 2 only (not hyperlinks)");
        assert!(rest.is_empty());
        assert!(titles_say_action_required(&titles));
        let (titles, rest) = osc_titles(b"x\x1b]0;[ ! ] Act");
        assert!(titles.is_empty());
        assert_eq!(rest, b"\x1b]0;[ ! ] Act", "an unfinished sequence is carried to the next chunk");
        let mut joined = rest.to_vec();
        joined.extend_from_slice(b"ion Required\x07");
        assert!(titles_say_action_required(&osc_titles(&joined).0));
        assert!(!titles_say_action_required(&["codex | t".to_string()]));
    }
}
