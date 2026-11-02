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
    Activity, HookReport, Signal, Watch, codex_approval_shown, needs_attention, next, osc_titles, program_status,
    program_status_app, squash, strip_ansi, tick_with, titles_say_action_required,
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
    /// Once Codex or Devin emits OSC 7501, only its explicit statuses drive activity for this terminal run.
    program_status_seen: bool,
    /// An interrupt key was sent; wait for the TUI acknowledgement before declaring the turn idle.
    interrupt_pending: bool,
    /// Ignore late tool hooks after an acknowledged interrupt until new work starts.
    interrupted: bool,
    /// Set when input resumes Codex, requests interruption, or starts a Claude turn: forget the scanned screen.
    clear_screen: bool,
    /// Tells this terminal run's tracker from a later one for the same session (restart).
    generation: u64,
    /// When someone last typed into the terminal (ms), so pdtd never types over a half-written prompt.
    last_input_ms: i64,
}

/// Recent agent output, owned by the session's output task (scanned outside any shared lock).
#[derive(Default)]
struct Screen {
    /// Squashed text, searched for approval and interruption messages.
    text: String,
    /// An unfinished window-title sequence from the previous chunk.
    title_carry: Vec<u8>,
    /// Separate from the heuristic screen: input must not discard a split program-status sequence.
    program_carry: Vec<u8>,
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

    fn program_status(&mut self, bytes: &[u8], app: &str) -> Option<Activity> {
        let mut raw = std::mem::take(&mut self.program_carry);
        raw.extend_from_slice(bytes);
        let (activity, rest) = program_status(&raw, app);
        if rest.len() <= TITLE_CARRY {
            self.program_carry = rest.to_vec();
        }
        activity
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
        map.get_mut(id).map(|t| transition(t, sig))
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
        persist(st, id, prev, now).await?;
    }
    Ok(())
}

/// Moves a tracked session's activity on by a signal: (before, after).
fn transition(t: &mut Tracker, sig: Signal) -> (Activity, Activity) {
    if matches!(sig, Signal::ProgramStatus(_)) {
        t.program_status_seen = true;
    } else if t.program_status_seen {
        // Includes Busy from input typed before the first explicit state was observed.
        return (t.activity, t.activity);
    }
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
}

/// Stores a transition and acts on it (call in the session's order).
async fn persist(st: &AppState, id: &str, prev: Activity, now: Activity) -> Result<(), ApiError> {
    st.store.update_activity(id, Some(now), needs_attention(prev, now).then(now_ms)).await?;
    crate::notifications::kick(st);
    crate::workflow::sensors::activity_changed(st, id, Some(prev), now);
    if now == Activity::Idle {
        // A terminal agent's turn ended: an agent step running in it may have stalled (Phase 7b spec §4.4).
        let (st2, id) = (st.clone(), id.to_string());
        tokio::spawn(async move { crate::workflow::agent::turn_ended(&st2, &id).await });
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
                program_status_seen: false,
                interrupt_pending: false,
                interrupted: false,
                generation,
                // A TUI that just started is still drawing: the quiet window starts now (PDT-129).
                last_input_ms: now_ms(),
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
    let (agent, activity, clear, interrupt_pending, program_status_seen) = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        t.watch.last_output_ms = now_ms();
        if t.watch.enter_at_ms.is_some() {
            t.watch.output_since_enter = true;
        }
        (t.agent, t.activity, std::mem::take(&mut t.clear_screen), t.interrupt_pending, t.program_status_seen)
    };
    if clear {
        screen.clear();
    }
    if let Some(app) = program_status_app(agent) {
        if let Some(activity) = screen.program_status(bytes, app) {
            let _ = apply(st, id, Signal::ProgramStatus(activity)).await;
            return;
        }
        if program_status_seen {
            return;
        }
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

/// Whether a chunk of terminal input is only what the terminal sent by itself: focus reports (`ESC [ I`, `ESC [ O`),
/// replies to queries (device attributes `ESC [ ? … c`, cursor position `ESC [ r ; c R`, mode reports `… $ y`,
/// keyboard flags `ESC [ ? … u`, status `ESC [ … n`) and OSC / DCS replies (colours, `XTGETTCAP`). The user typing
/// sends anything else.
fn is_terminal_reply(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            return false;
        }
        match bytes.get(i + 1) {
            Some(b'[') => {
                let start = i + 2;
                let mut j = start;
                while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                    j += 1;
                }
                let Some(&last) = bytes.get(j) else { return false };
                let params = &bytes[start..j];
                let private = params.first().is_some_and(|b| matches!(b, b'?' | b'>' | b'='));
                let reply = match last {
                    b'I' | b'O' => params.is_empty(),
                    b'c' => private,
                    b'u' => params.first() == Some(&b'?'),
                    b'R' | b'n' => !params.is_empty(),
                    b'y' => params.last() == Some(&b'$'),
                    _ => false,
                };
                if !reply {
                    return false;
                }
                i = j + 1;
            }
            Some(b']' | b'P') => {
                let mut j = i + 2;
                i = loop {
                    match bytes.get(j) {
                        None => return false,
                        Some(0x07) => break j + 1,
                        Some(0x1b) if bytes.get(j + 1) == Some(&b'\\') => break j + 2,
                        Some(_) => j += 1,
                    }
                };
            }
            _ => return false,
        }
    }
    !bytes.is_empty()
}

/// A plain Enter submits input; Alt/Shift+Enter (`ESC CR`) inserts a composer newline.
fn is_enter(bytes: &[u8]) -> bool {
    bytes.iter().enumerate().any(|(i, b)| *b == b'\r' && (i == 0 || bytes[i - 1] != 0x1b))
}

/// Whether pdtd may type a message into the agent session now: it runs in this daemon, is idle (not working, not
/// asking the user anything) and nobody has typed into it for `quiet_ms`.
pub(crate) fn ready_for_message(st: &AppState, id: &str, quiet_ms: i64) -> bool {
    let map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
    map.get(id).is_some_and(|t| t.activity == Activity::Idle && now_ms() - t.last_input_ms >= quiet_ms)
}

/// A tracked agent terminal's activity (None: this daemon does not track it).
pub(crate) fn activity_of(st: &AppState, id: &str) -> Option<Activity> {
    st.activity.lock().unwrap_or_else(|e| e.into_inner()).get(id).map(|t| t.activity)
}

/// The agent terminals this daemon tracks whose activity is `working` (the host's cap counts them, Phase 7c §6.2).
pub(crate) fn working_ids(st: &AppState) -> Vec<String> {
    let map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
    map.iter().filter(|(_, t)| t.activity == Activity::Working).map(|(id, _)| id.clone()).collect()
}

/// Types `text` into an agent session as one bracketed paste, then presses Enter (as the browser's Compose does), so
/// the agent takes it as a prompt. Activity sees it as input. Fails when the terminal's process is gone before the
/// paste or the Enter (PDT-129): the bytes would reach nobody. (One that exits once it has read the line took it.)
pub(crate) async fn type_message(st: &AppState, id: &str, text: &str) -> Result<(), pdt_agents::PtyError> {
    let body = text.replace('\x1b', "").replace("\r\n", "\r").replace('\n', "\r");
    let paste = format!("\x1b[200~{body}\x1b[201~");
    st.ptys.write(id, paste.as_bytes())?;
    // Some TUIs treat an Enter in the same chunk as part of the paste.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    on_input(st, id, b"\r");
    st.ptys.write(id, b"\r")
}

/// Called with every chunk the user types into a terminal (no-op for terminals without a tracker).
pub(crate) fn on_input(st: &AppState, id: &str, bytes: &[u8]) {
    let enter = is_enter(bytes);
    let (prev, now) = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        // What the terminal answers by itself (a focus change, a query's reply) is not someone typing (PDT-129).
        if is_terminal_reply(bytes) {
            return;
        }
        t.last_input_ms = now_ms();
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
        let sig = match t.agent {
            // Legacy Codex fallback. Once OSC 7501 is seen, Enter can also be trust, menus, or empty input.
            AgentKind::Codex if !t.program_status_seen && (enter || answered) => {
                t.clear_screen = true;
                Signal::Busy
            }
            // Claude reports its own turns, but answering a permission prompt sends no hook until its next tool.
            AgentKind::Claude if answered => Signal::Busy,
            _ => return,
        };
        // Applied now, before any output the input causes is scanned: a task applying it later could land after
        // the agent's next prompt and undo it (PDT-259). Only the store write waits for the session's order.
        transition(t, sig)
    };
    if prev != now {
        let (st, id) = (st.clone(), id.to_string());
        tokio::spawn(async move {
            let order = order_lock(&st, &id);
            let _in_order = order.lock().await;
            // A later transition already stored (or will store) the newer state.
            if activity_of(&st, &id) == Some(now) {
                let _ = persist(&st, &id, prev, now).await;
            }
        });
    }
}

/// Applies time-based signals (`tick`) to every tracked session every `activity_tick`, until shutdown.
pub async fn run_activity_ticker(st: AppState) {
    let every = st.activity_tick;
    crate::loops::run_every(
        st,
        "activity",
        every,
        crate::loops::Start::Now,
        |st| async move { activity_pass(&st).await },
    )
    .await
}

async fn activity_pass(st: &AppState) {
    let due: Vec<(String, Signal)> = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms();
        map.iter_mut()
            .filter_map(|(id, t)| {
                if t.program_status_seen {
                    return None;
                }
                let sig = tick_with(t.activity, &t.watch, t.agent == AgentKind::Claude, now, &st.activity_timing)?;
                if sig == Signal::HookMissing {
                    t.watch.enter_at_ms = None;
                }
                Some((id.clone(), sig))
            })
            .collect()
    };
    for (id, sig) in due {
        let _ = apply(st, &id, sig).await;
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
    if let HookReport::Devin { session_id: Some(conversation), .. } = &report {
        remember_devin_conversation(&st, &s, conversation).await;
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
    drop(_in_order);
    // `/clear` (or `/resume`) in the TUI starts another conversation: restarts must resume that one.
    if let HookReport::Claude { event, session_id: Some(new_id), start_source, .. } = &report
        && event == "SessionStart"
        && s.agent == Some(AgentKind::Claude)
        && new_id.as_str() != s.claude_conversation_id()
        && pdt_core::session::is_claude_conversation_id(new_id)
    {
        let trigger = start_source.as_deref().unwrap_or("clear");
        crate::conversations::terminal_conversation_moved(&st, &s, new_id, trigger).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Records the Devin conversation a session's terminal runs, as its `SessionStart` hook names it: a fresh launch
/// gets a new one, and resuming one Devin never saved (no prompt yet) quietly starts a new one too, so the latest
/// report always wins.
async fn remember_devin_conversation(st: &AppState, s: &Session, conversation: &str) {
    let valid = !conversation.is_empty()
        && conversation.len() <= 200
        && conversation.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if s.agent != Some(AgentKind::Devin) || !valid || s.external_session_id.as_deref() == Some(conversation) {
        return;
    }
    match st.store.set_external_session_id(&s.id, conversation).await {
        Ok(_) => tracing::info!(session = %s.id, conversation, "the session runs this Devin conversation"),
        // Another session already runs it (the user resumed it in both): this one keeps its own.
        Err(e) => tracing::warn!(session = %s.id, conversation, error = %e, "could not record the Devin conversation"),
    }
}

pub(crate) async fn post_view(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let now = now_ms();
    if !st.store.mark_viewed(&id, now).await? {
        return Err(ApiError::not_found("session"));
    }
    // Viewing a session reads its notifications; the sync clears the ones its state no longer calls for.
    st.store.read_notifications(None, Some(&id), None, now).await?;
    crate::notifications::kick(&st);
    Ok(StatusCode::NO_CONTENT)
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/sessions/{id}/activity", post(post_report))
        .route("/sessions/{id}/view", post(post_view))
}

#[cfg(test)]
mod tests {
    use super::{
        Screen, Tracker, activity_of, apply_ordered, is_answer, is_enter, is_terminal_reply, on_input, order_lock,
    };
    use crate::routes::AppState;
    use pdt_core::activity::{Activity, Signal};
    use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};

    /// PDT-259: answering Codex's prompt and its next prompt arriving while the session's ordering point is busy
    /// (a store write, a loaded machine) must not end with the answer applied last, undoing the new prompt.
    #[tokio::test]
    async fn input_is_applied_before_the_output_it_causes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let st = AppState::new(store.clone(), tmp.path().join("data"));
        let repo = tmp.path().join("repo");
        let project = pdt_core::project::Project {
            id: "p".into(),
            name: "P".into(),
            repo_path: repo.to_string_lossy().into(),
            base_branch: "main".into(),
            created_at: 1,
            key: String::new(),
            forge: None,
            kind: Default::default(),
        };
        store.insert_project(&project).await.unwrap();
        store
            .insert_session(&Session {
                id: "c".into(),
                project_id: project.id.clone(),
                worktree_id: None,
                kind: SessionKind::AgentTui,
                agent: Some(AgentKind::Codex),
                bypass_permissions: false,
                title: "t".into(),
                cwd: repo.to_string_lossy().into(),
                status: SessionStatus::Running,
                exit_code: None,
                created_at: 1,
                ended_at: None,
                external_session_id: None,
                fresh_started_at: None,
                activity: Some(Activity::NeedsYou),
                attention_at: None,
                last_viewed_at: None,
                unseen: false,
                attempt_id: None,
                ticket: None,
                mode: pdt_core::conversation::SessionMode::Terminal,
                model: None,
                permission_mode: None,
                persona: None,
                background: vec![],
            })
            .await
            .unwrap();
        st.activity.lock().unwrap().insert(
            "c".into(),
            Tracker {
                agent: AgentKind::Codex,
                activity: Activity::NeedsYou,
                watch: Default::default(),
                program_status_seen: false,
                interrupt_pending: false,
                interrupted: false,
                clear_screen: false,
                generation: 0,
                last_input_ms: 0,
            },
        );
        let order = order_lock(&st, "c");
        let busy = order.lock().await;
        on_input(&st, "c", b"y\r");
        assert_eq!(activity_of(&st, "c"), Some(Activity::Working), "the answer counts at once");
        // The output task gets the ordering point first and sees the next prompt.
        apply_ordered(&st, "c", Signal::NeedsYou).await.unwrap();
        drop(busy);
        for _ in 0..20 {
            tokio::task::yield_now().await;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(activity_of(&st, "c"), Some(Activity::NeedsYou));
        assert_eq!(store.get_session("c").await.unwrap().unwrap().activity, Some(Activity::NeedsYou));
    }

    #[test]
    fn terminal_replies_are_not_typing() {
        for sent in [
            &b"\x1b[I"[..],
            b"\x1b[O",
            b"\x1b[I\x1b[O",
            b"\x1b[?1;2c",
            b"\x1b[>0;276;0c",
            b"\x1b[12;40R",
            b"\x1b[0n",
            b"\x1b[?2004;1$y",
            b"\x1b[?0u",
            b"\x1b]11;rgb:0000/0000/0000\x1b\\",
            b"\x1b]10;rgb:ffff/ffff/ffff\x07",
            b"\x1bP1+r544e=787465726d\x1b\\",
        ] {
            assert!(is_terminal_reply(sent), "{sent:?}");
        }
        for typed in [&b"a"[..], b"\r", b"\x1b", b"\x7f", b"\x1b[A", b"\x1b[3~", b"\x1b[I hi", b"", b"\x1b]11;unended"]
        {
            assert!(!is_terminal_reply(typed), "{typed:?}");
        }
    }

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
