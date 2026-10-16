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
    Activity, CODEX_APPROVAL_MARKERS, HookReport, Signal, Watch, needs_attention, next, screen_shows, squash,
    strip_ansi, tick,
};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast::error::RecvError;

/// How much squashed Codex output is kept to look for the approval prompt (it arrives in pieces).
const SCREEN_TAIL: usize = 16 * 1024;
static GENERATION: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Tracker {
    agent: AgentKind,
    activity: Activity,
    watch: Watch,
    /// Squashed recent output (Codex), searched for the approval prompt.
    screen: String,
    /// Tells this terminal run's tracker from a later one for the same session (restart).
    generation: u64,
}

/// Applies a signal to a session: to its tracker when it has one (running in this daemon), else to the stored row
/// of a running agent session. Records attention when it becomes idle or needs the user.
pub(crate) async fn apply(st: &AppState, id: &str, sig: Signal) -> Result<(), ApiError> {
    let tracked = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        map.get_mut(id).map(|t| {
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
    st.activity.lock().unwrap_or_else(|e| e.into_inner()).insert(
        s.id.clone(),
        Tracker {
            agent,
            activity: Activity::Idle,
            watch: Watch { last_output_ms: now_ms(), ..Default::default() },
            screen: String::new(),
            generation,
        },
    );
    let _ = st.store.update_activity(&s.id, Some(Activity::Idle), None).await;
    let (st, id) = (st.clone(), s.id.clone());
    tokio::spawn(async move {
        // What it printed before the tracker attached.
        if !att.scrollback.is_empty() {
            on_output(&st, &id, &att.scrollback).await;
        }
        let mut events = att.events;
        loop {
            match events.recv().await {
                Ok(PtyEvent::Output(bytes)) => on_output(&st, &id, &bytes).await,
                Ok(PtyEvent::Exit(_)) | Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            }
        }
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

async fn on_output(st: &AppState, id: &str, bytes: &[u8]) {
    let sig = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        t.watch.last_output_ms = now_ms();
        if t.watch.enter_at_ms.is_some() {
            t.watch.output_since_enter = true;
        }
        if t.agent != AgentKind::Codex || t.activity == Activity::NeedsYou {
            return;
        }
        t.screen.push_str(&squash(&strip_ansi(bytes)));
        if t.screen.len() > SCREEN_TAIL {
            let mut cut = t.screen.len() - SCREEN_TAIL;
            while !t.screen.is_char_boundary(cut) {
                cut += 1;
            }
            t.screen.drain(..cut);
        }
        if !screen_shows(&t.screen, CODEX_APPROVAL_MARKERS) {
            return;
        }
        t.screen.clear();
        Signal::NeedsYou
    };
    let _ = apply(st, id, sig).await;
}

/// Whether terminal input is the user pressing a key, rather than something the terminal sent by itself (focus
/// reports, replies to queries) or a cursor/arrow key: Enter, a lone Esc, or printable text.
fn is_answer(bytes: &[u8]) -> bool {
    bytes.contains(&b'\r') || bytes == b"\x1b" || !strip_ansi(bytes).trim().is_empty()
}

/// Called with every chunk the user types into a terminal (no-op for terminals without a tracker).
pub(crate) fn on_input(st: &AppState, id: &str, bytes: &[u8]) {
    let enter = bytes.contains(&b'\r');
    let sig = {
        let mut map = st.activity.lock().unwrap_or_else(|e| e.into_inner());
        let Some(t) = map.get_mut(id) else { return };
        match t.agent {
            // Codex has no per-prompt hook: Enter starts a turn, and answering its approval prompt resumes it.
            AgentKind::Codex if enter || (t.activity == Activity::NeedsYou && is_answer(bytes)) => {
                t.screen.clear();
                Some(Signal::Busy)
            }
            // Claude reports its own turns; Enter only starts the wait for that hook (see `tick`).
            AgentKind::Claude if enter => {
                t.watch.enter_at_ms = Some(now_ms());
                t.watch.hook_since_enter = false;
                t.watch.output_since_enter = false;
                None
            }
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

/// Applies time-based signals (`tick`) to every tracked session every 5 s, until shutdown.
pub async fn run_activity_ticker(st: AppState) {
    let mut every = tokio::time::interval(std::time::Duration::from_secs(5));
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
                    let sig = tick(t.activity, &t.watch, t.agent == AgentKind::Claude, now)?;
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
    if let Some(t) = st.activity.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&id) {
        t.watch.hook_since_enter = true;
        t.watch.enter_at_ms = None;
    }
    if let Some(sig) = report.signal() {
        apply(&st, &id, sig).await?;
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
    use super::is_answer;

    #[test]
    fn answers_are_keys_the_user_pressed() {
        for typed in [&b"y"[..], b"\r", b"\x1b", b"2", b"no\r"] {
            assert!(is_answer(typed), "{typed:?}");
        }
        for sent in [&b"\x1b[I"[..], b"\x1b[O", b"\x1b[B", b"\x1b[?1;2c", b"\x1b]11;rgb:0000/0000/0000\x1b\\"] {
            assert!(!is_answer(sent), "{sent:?}");
        }
    }
}
