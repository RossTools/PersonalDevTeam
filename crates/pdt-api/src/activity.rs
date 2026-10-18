//! Agent activity (remote-host spec §5): hook reports from `pdtd hook`, terminal input and output heuristics, and
//! the unseen flag. Only agent sessions are tracked; shells and run terminals never are.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use pdt_agents::PtyEvent;
use pdt_core::activity::{
    Activity, HookReport, Signal, Watch, codex_approval_shown, needs_attention, next, osc_titles, squash, strip_ansi,
    tick_with, titles_say_action_required,
};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast::error::RecvError;

/// How much squashed output is kept to look for agent screen markers (they arrive in pieces).
const SCREEN_TAIL: usize = 16 * 1024;
/// The longest unfinished window-title sequence carried from one output chunk to the next.
const TITLE_CARRY: usize = 1024;
static GENERATION: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Tracker {
    agent: AgentKind,
    activity: Activity,
    watch: Watch,
    /// An interrupt key was sent; wait for the TUI acknowledgement before declaring the turn idle.
    interrupt_pending: bool,
    /// Ignore late tool hooks after an acknowledged interrupt until new work starts.
    interrupted: bool,
    /// Set when input resumes Codex, requests interruption, or starts a Claude turn: forget the scanned screen.
    clear_screen: bool,
    /// Tells this terminal run's tracker from a later one for the same session (restart).
    generation: u64,
}

/// Recent agent output, owned by the session's output task (scanned outside any shared lock).
#[derive(Default)]
struct Screen {
    /// Squashed text, searched for approval and interruption messages.
    text: String,
    /// An unfinished window-title sequence from the previous chunk.
    title_carry: Vec<u8>,
}

impl Screen {
    fn clear(&mut self) {
        self.text.clear();
        self.title_carry.clear();
    }

    /// Adds a chunk of output; true when it now shows Codex's approval prompt.
    fn shows_approval(&mut self, bytes: &[u8]) -> bool {
        let mut raw = std::mem::take(&mut self.title_carry);
        raw.extend_from_slice(bytes);
        let (titles, rest) = osc_titles(&raw);
        let title = titles_say_action_required(&titles);
        if rest.len() <= TITLE_CARRY {
            self.title_carry = rest.to_vec();
        }
        self.push_text(bytes);
        codex_approval_shown(title, &self.text)
    }

    fn shows_interruption(&mut self, bytes: &[u8]) -> bool {
        self.push_text(bytes);
        self.text.contains("interrupted") && self.text.contains("whatshouldclaudedoinstead?")
    }

    fn push_text(&mut self, bytes: &[u8]) {
        self.text.push_str(&squash(&strip_ansi(bytes)));
        if self.text.len() > SCREEN_TAIL {
            let mut cut = self.text.len() - SCREEN_TAIL;
            while !self.text.is_char_boundary(cut) {
                cut += 1;
            }
            self.text.drain(..cut);
        }
    }
}

/// The session's activity ordering point: transitions and their store writes happen one at a time, in order, so the
/// row always ends with the latest state.
fn order_lock(st: &AppState, id: &str) -> Arc<tokio::sync::Mutex<()>> {
    st.activity_order.lock().unwrap_or_else(|e| e.into_inner()).entry(id.to_string()).or_default().clone()
}

/// Forgets a removed session's ordering point.
pub(crate) fn forget(st: &AppState, id: &str) {
    st.activity_order.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
}

/// Applies a signal to a session: to its tracker when it has one (running in this daemon), else to the stored row
/// of a running agent session. Records attention when it becomes idle or needs the user.
pub(crate) async fn apply(st: &AppState, id: &str, sig: Signal) -> Result<(), ApiError> {
    let order = order_lock(st, id);
    let _in_order = order.lock().await;
    apply_ordered(st, id, sig).await
}

async fn apply_ordered(st: &AppState, id: &str, sig: Signal) -> Result<(), ApiError> {
    let tracked = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        map.get_mut(id).map(|t| {
            if sig == Signal::Interrupted {
                if !t.interrupt_pending {
                    return (t.activity, t.activity);
                }
                t.interrupt_pending = false;
                t.interrupted = true;
                t.watch.enter_at_ms = None;
            } else if matches!(sig, Signal::Busy | Signal::NeedsYou) && t.interrupted {
                return (t.activity, t.activity);
            } else if sig == Signal::TurnEnded {
                t.interrupt_pending = false;
            }
            let prev = t.activity;
            t.activity = next(prev, sig);
            (prev, t.activity)
        })
    };
    let (prev, now) = match tracked {
        Some(p) => p,
        None => {
            let Some(s) = st.store.get_session(id).await? else { return Ok(()) };
            // An ended session has no activity; a hook arriving as it exits must not bring one back.
            if s.status != SessionStatus::Running || s.kind != SessionKind::AgentTui {
                return Ok(());
            }
            let prev = s.activity.unwrap_or(Activity::Idle);
            (prev, next(prev, sig))
        }
    };
    if prev != now {
        st.store.update_activity(id, Some(now), needs_attention(prev, now).then(now_ms)).await?;
    }
    Ok(())
}

/// Starts tracking an agent session whose terminal just started: idle, watching its output until it exits.
pub(crate) async fn track(st: &AppState, s: &Session) {
    let Some(agent) = s.agent.filter(|_| s.kind == SessionKind::AgentTui) else { return };
    let Ok(att) = st.ptys.attach(&s.id) else { return };
    if att.exited.is_some() {
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst);
    {
        let order = order_lock(st, &s.id);
        let _in_order = order.lock().await;
        st.activity.lock().unwrap_or_else(|e| e.into_inner()).insert(
            s.id.clone(),
            Tracker {
                agent,
                activity: Activity::Idle,
                watch: Watch { last_output_ms: now_ms(), ..Default::default() },
                clear_screen: false,
                interrupt_pending: false,
                interrupted: false,
                generation,
            },
        );
        let _ = st.store.update_activity(&s.id, Some(Activity::Idle), None).await;
    }
    let (st, id) = (st.clone(), s.id.clone());
    tokio::spawn(async move {
        let mut screen = Screen::default();
        // What it printed before the tracker attached.
        if !att.scrollback.is_empty() {
            on_output(&st, &id, &att.scrollback, &mut screen).await;
        }
        let mut events = att.events;
        loop {
            match events.recv().await {
                Ok(PtyEvent::Output(bytes)) => on_output(&st, &id, &bytes, &mut screen).await,
                Ok(PtyEvent::Exit(_)) | Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            }
        }
        let order = order_lock(&st, &id);
        let _in_order = order.lock().await;
        let mine = {
            let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
            let mine = map.get(&id).is_some_and(|t| t.generation == generation);
            if mine {
                map.remove(&id);
            }
            mine
        };
        if mine {
            let _ = st.store.update_activity(&id, None, None).await;
        }
    });
}

async fn on_output(st: &AppState, id: &str, bytes: &[u8], screen: &mut Screen) {
    let (agent, activity, clear, interrupt_pending) = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        t.watch.last_output_ms = now_ms();
        if t.watch.enter_at_ms.is_some() {
            t.watch.output_since_enter = true;
        }
        (t.agent, t.activity, std::mem::take(&mut t.clear_screen), t.interrupt_pending)
    };
    if clear {
        screen.clear();
    }
    let signal = match agent {
        AgentKind::Claude if interrupt_pending && screen.shows_interruption(bytes) => Signal::Interrupted,
        AgentKind::Codex if activity != Activity::NeedsYou && screen.shows_approval(bytes) => Signal::NeedsYou,
        _ => return,
    };
    screen.clear();
    let _ = apply(st, id, signal).await;
}

/// Whether terminal input is the user pressing a key, rather than something the terminal sent by itself (focus
/// reports, replies to queries) or a cursor/arrow key: Enter, a lone Esc, or printable text.
fn is_answer(bytes: &[u8]) -> bool {
    bytes.contains(&b'\r') || bytes == b"\x1b" || !strip_ansi(bytes).trim().is_empty()
}

/// A plain Enter submits input; Alt/Shift+Enter (`ESC CR`) inserts a composer newline.
fn is_enter(bytes: &[u8]) -> bool {
    bytes.iter().enumerate().any(|(i, b)| *b == b'\r' && (i == 0 || bytes[i - 1] != 0x1b))
}

/// Called with every chunk the user types into a terminal (no-op for terminals without a tracker).
pub(crate) fn on_input(st: &AppState, id: &str, bytes: &[u8]) {
    let enter = is_enter(bytes);
    let sig = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        if t.agent == AgentKind::Claude && matches!(bytes, b"\x1b" | b"\x03") {
            // Esc can also close a dialog. Only the subsequent acknowledgement proves the turn stopped.
            t.interrupt_pending = true;
            t.clear_screen = true;
            return;
        }
        let answered = t.activity == Activity::NeedsYou && is_answer(bytes);
        if t.agent == AgentKind::Claude && enter && !t.watch.hook_seen {
            // Until a hook has arrived, an Enter that prints but brings no hook within the grace period means the
            // session's hooks are broken (see `tick`).
            t.watch.enter_at_ms = Some(now_ms());
            t.watch.output_since_enter = false;
        }
        match t.agent {
            // Codex has no per-prompt hook: Enter starts a turn, and answering its approval prompt resumes it.
            AgentKind::Codex if enter || answered => {
                t.clear_screen = true;
                Some(Signal::Busy)
            }
            // Claude reports its own turns, but answering a permission prompt sends no hook until its next tool.
            AgentKind::Claude if answered => Some(Signal::Busy),
            _ => None,
        }
    };
    if let Some(sig) = sig {
        let (st, id) = (st.clone(), id.to_string());
        tokio::spawn(async move {
            let _ = apply(&st, &id, sig).await;
        });
    }
}

/// Applies time-based signals (`tick`) to every tracked session every `activity_tick`, until shutdown.
pub async fn run_activity_ticker(st: AppState) {
    let mut every = tokio::time::interval(st.activity_tick);
    loop {
        every.tick().await;
        if st.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let due: Vec<(String, Signal)> = {
            let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
            let now = now_ms();
            map.iter_mut()
                .filter_map(|(id, t)| {
                    let sig = tick_with(t.activity, &t.watch, t.agent == AgentKind::Claude, now, &st.activity_timing)?;
                    if sig == Signal::HookMissing {
                        t.watch.enter_at_ms = None;
                    }
                    Some((id.clone(), sig))
                })
                .collect()
        };
        for (id, sig) in due {
            let _ = apply(&st, &id, sig).await;
        }
    }
}

pub(crate) async fn post_report(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<HookReport>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let Json(report) = payload?;
    let s = st.store.get_session(&id).await?.ok_or_else(|| ApiError::not_found("session"))?;
    if s.kind != SessionKind::AgentTui {
        return Err(ApiError::new(StatusCode::CONFLICT, "not_an_agent", "only agent sessions report activity"));
    }
    let order = order_lock(&st, &id);
    let _in_order = order.lock().await;
    if let Some(t) = st.activity.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&id) {
        if matches!(&report, HookReport::Claude { event, .. }
            if event == "UserPromptSubmit" || (event == "PreToolUse" && t.interrupted))
        {
            t.interrupted = false;
            t.interrupt_pending = false;
            t.clear_screen = true;
        }
        t.watch.hook_seen = true;
        t.watch.enter_at_ms = None;
    }
    if let Some(sig) = report.signal() {
        apply_ordered(&st, &id, sig).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn post_view(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    if !st.store.mark_viewed(&id, now_ms()).await? {
        return Err(ApiError::not_found("session"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::{Screen, is_answer, is_enter};

    #[test]
    fn claude_interruption_acknowledgement_is_found_across_chunks() {
        let mut s = Screen::default();
        assert!(!s.shows_interruption(b"\x1b[31mInterrupted\x1b[0m"));
        assert!(!s.shows_interruption(b" \xc2\xb7 What should Claude do "));
        assert!(s.shows_interruption(b"instead?"));
        s.clear();
        assert!(!s.shows_interruption(b"What should Claude do instead?"), "also needs Interrupted");
    }

    #[test]
    fn composing_a_newline_does_not_start_a_codex_turn() {
        assert!(is_enter(b"\r"));
        assert!(is_enter(b"prompt\r"));
        assert!(!is_enter(b"\x1b\r"), "Shift+Enter inserts a newline in the composer");
        assert!(!is_enter(b"\x1b[I"), "focus reports do not submit input");
    }

    #[test]
    fn answers_are_keys_the_user_pressed() {
        for typed in [&b"y"[..], b"\r", b"\x1b", b"2", b"no\r"] {
            assert!(is_answer(typed), "{typed:?}");
        }
        for sent in [&b"\x1b[I"[..], b"\x1b[O", b"\x1b[B", b"\x1b[?1;2c", b"\x1b]11;rgb:0000/0000/0000\x1b\\"] {
            assert!(!is_answer(sent), "{sent:?}");
        }
    }

    #[test]
    fn the_approval_prompt_is_found_across_chunks() {
        let mut s = Screen::default();
        assert!(!s.shows_approval(b"Here is the plan. Yes, proceed when ready.\r\n"), "one marker is not enough");
        assert!(!s.shows_approval(b"\x1b]0;[ ! ] Action Req"));
        assert!(s.shows_approval(b"uired | t | f\x07"), "the title split over two chunks");
        s.clear();
        assert!(!s.shows_approval(b"Would you like to run the following "));
        assert!(s.shows_approval(b"command?\r\n  1. Yes, proceed (y)"), "two markers, the first split");
    }
}
