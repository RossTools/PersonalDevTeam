//! Agents that aren't logged in (PDT-604). An agent's error that says so (`pdt_core::agent_login`), confirmed by its
//! login check when the words alone aren't sure, opens one Inbox item for the agent; while it is open the agent counts
//! as logged out: the step whose turn failed waits instead of being nudged, and new steps, pickup and scheduled steps
//! that would use the agent wait too (`workflow::agent::login`). The login is checked again every few minutes while
//! the item is open, and on Check again; once the agent is logged in, the waiting steps go on and the item closes,
//! saying how many did. The open item is the record, so it holds over a restart.

use crate::error::{ApiError, codes};
use crate::routes::{AppState, now_ms};
use crate::workflow::agent::login;
use pdt_core::agent_login::{self as core, Strength};
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::session::AgentKind;
use serde_json::{Value, json};
use std::time::Duration;

/// How often the login of an agent with an open item is checked again.
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(120);
/// The item's choice.
pub(crate) const CHECK_AGAIN: &str = "check_again";
/// How much of the agent's error the item keeps.
const DETAIL_CHARS: usize = 500;

/// The agent's name: a built-in's, or an ACP agent's label.
pub(crate) async fn label(st: &AppState, agent: AgentKind) -> String {
    match st.store.get_host_settings().await {
        Ok(h) => h.agent_label(agent),
        Err(_) => agent.label().to_string(),
    }
}

/// The agent the item is about.
fn agent_of(item: &InboxItem) -> Option<AgentKind> {
    item.body.get("agent").and_then(Value::as_str).and_then(AgentKind::parse_stored)
}

/// The open item of an agent that isn't logged in.
pub(crate) async fn open_item(st: &AppState, agent: AgentKind) -> Option<InboxItem> {
    let items = st.store.open_inbox_items_of(InboxKind::AgentLogin).await.ok()?;
    items.into_iter().find(|i| agent_of(i) == Some(agent))
}

/// Whether the agent is known not to be logged in: its item is open.
pub(crate) async fn logged_out(st: &AppState, agent: AgentKind) -> bool {
    open_item(st, agent).await.is_some()
}

/// A session's error said its agent isn't logged in. Once that is sure (the error's own words, else the agent's login
/// check), the agent's item is open and the session's step waits for the login. Whether it was sure: else the turn is
/// one like any other.
pub(crate) async fn failed(st: &AppState, session_id: &str, error: &str) -> bool {
    let Ok(Some(s)) = st.store.get_session(session_id).await else { return false };
    let agent = s.agent.unwrap_or(AgentKind::Claude);
    let Some(strength) = core::login_failure(agent, error) else { return false };
    {
        // Sessions of one agent fail together: one check, one item. The step is held under the same lock, so a resume
        // can't close the item between the two and leave it waiting on nothing.
        let _one = st.agent_login_locks.lock(agent.as_str()).await;
        if open_item(st, agent).await.is_none() {
            let logged_in = crate::checks::agent_logged_in(st, agent).await;
            if !core::confirmed(strength, logged_in) {
                tracing::info!(session = %session_id, agent = agent.as_str(), "an agent's error reads like a login \
                    failure, but its login check doesn't agree");
                return false;
            }
            open(st, agent, error, strength == Strength::Sure && logged_in == Some(true)).await;
        }
        login::hold(st, session_id).await;
    }
    crate::notifications::kick(st);
    true
}

/// Opens the agent's item. `blind`: its login check says it is logged in all the same (an expired token it can't
/// see), so only Check again, not the periodic check, takes the steps up again.
async fn open(st: &AppState, agent: AgentKind, error: &str, blind: bool) {
    let label = label(st, agent).await;
    let detail: String = error.trim().chars().take(DETAIL_CHARS).collect();
    let body = json!({"agent": agent.as_str(), "label": label, "hint": core::login_hint(agent, &label),
                      "detail": detail, "blind": blind, "waiting": 0, "steps": []});
    let item = crate::inbox::new_item(InboxKind::AgentLogin, InboxGroup::Notice, core::title(&label, 0), body)
        .choices(vec![InboxChoice::new(CHECK_AGAIN, "Check again", true)]);
    match st.store.open_inbox_item(&item).await {
        Ok(_) => tracing::warn!(agent = agent.as_str(), "an agent isn't logged in: its steps wait for it"),
        Err(e) => tracing::warn!(agent = agent.as_str(), error = %e, "could not put an agent's login in the Inbox"),
    }
}

/// The item as listed: the steps waiting now, counted in its title.
pub(crate) async fn decorate(st: &AppState, item: &mut InboxItem) {
    let Some(agent) = agent_of(item) else { return };
    let waiting = login::waiting(st, agent).await;
    let mut steps = vec![];
    for w in &waiting {
        let ticket = match &w.ticket_id {
            Some(t) => st.store.get_ticket_summary(t).await.ok().flatten().map(|t| t.key),
            None => None,
        };
        steps.push(json!({"stepId": w.step_id, "persona": w.persona, "ticketId": w.ticket_id, "ticketKey": ticket,
                          "started": w.stopped.is_some()}));
    }
    let label =
        item.body.get("label").and_then(Value::as_str).map(String::from).unwrap_or_else(|| agent.label().into());
    item.title = core::title(&label, waiting.len());
    item.body["waiting"] = json!(waiting.len());
    item.body["steps"] = json!(steps);
}

/// Check again on the item: the agent's login is checked, and once it is logged in (or can't be checked), the steps
/// waiting for it go on and the item closes.
pub(crate) async fn answer(st: &AppState, item: &InboxItem, choice: Option<&str>) -> Result<Value, ApiError> {
    if choice != Some(CHECK_AGAIN) {
        return Err(ApiError::conflict(codes::CANNOT_ANSWER, "choose Check again"));
    }
    let agent = agent_of(item).ok_or_else(|| ApiError::conflict(codes::CANNOT_ANSWER, "the item names no agent"))?;
    let label = label(st, agent).await;
    if crate::checks::agent_logged_in(st, agent).await == Some(false) {
        let hint = core::login_hint(agent, &label);
        return Err(ApiError::conflict(codes::CANNOT_ANSWER, format!("{label} still isn't logged in. {hint}")));
    }
    let resumed = resume(st, agent, item, "user").await;
    Ok(json!({"loggedIn": true, "resumed": resumed}))
}

/// The agent is logged in again: the steps waiting for it go on, and its item closes saying how many did.
async fn resume(st: &AppState, agent: AgentKind, item: &InboxItem, by: &str) -> usize {
    let _one = st.agent_login_locks.lock(agent.as_str()).await;
    // Closed first: a step that goes on must not find the agent logged out and wait again.
    let label = label(st, agent).await;
    let waiting = login::waiting(st, agent).await;
    let count = waiting.len();
    let resolution = json!({"by": by, "resumed": count, "note": resumed_note(count)});
    match st.store.end_inbox_item(&item.id, InboxStatus::Closed, &resolution, now_ms()).await {
        Ok(true) => {}
        // Closed meanwhile (a check and Check again at once): its steps went on then.
        Ok(false) => return 0,
        Err(e) => {
            tracing::warn!(agent = agent.as_str(), error = %e, "could not close an agent's login item");
            return 0;
        }
    }
    let mut resumed = 0;
    for w in waiting {
        if login::go_on(st, w, &label).await {
            resumed += 1;
        }
    }
    crate::notifications::kick(st);
    tracing::info!(agent = agent.as_str(), resumed, "an agent is logged in again: its steps went on");
    resumed
}

/// "resumed 3 steps".
fn resumed_note(n: usize) -> String {
    match n {
        1 => "resumed 1 step".into(),
        n => format!("resumed {n} steps"),
    }
}

/// One pass of the login watch: each agent with an open item whose login check now says it is logged in has its
/// steps go on. One whose check said logged in all along (`blind`) waits for Check again. A step that waits for an
/// agent with no open item (held by a new step's start just as the item closed) goes on too.
pub async fn watch_logins(st: &AppState) {
    let Ok(items) = st.store.open_inbox_items_of(InboxKind::AgentLogin).await else { return };
    for agent in login::waiting_agents(st).await {
        let _one = st.agent_login_locks.lock(agent.as_str()).await;
        if open_item(st, agent).await.is_some() {
            continue;
        }
        let label = label(st, agent).await;
        for w in login::waiting(st, agent).await {
            tracing::info!(step = %w.step_id, agent = agent.as_str(), "a step waited for a login that no item holds");
            login::go_on(st, w, &label).await;
        }
    }
    for item in items {
        let Some(agent) = agent_of(&item) else { continue };
        if item.body.get("blind").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if crate::checks::agent_logged_in(st, agent).await == Some(true) {
            resume(st, agent, &item, "check").await;
        }
    }
}

/// pdtd's login watch loop.
pub async fn run_login_watch(st: AppState) {
    use crate::loops::{Start, run_every};
    run_every(st, "agent-login", CHECK_EVERY, Start::AfterOnePeriod, |st| async move { watch_logins(&st).await }).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_note_counts_the_steps() {
        assert_eq!(super::resumed_note(1), "resumed 1 step");
        assert_eq!(super::resumed_note(3), "resumed 3 steps");
        assert_eq!(super::resumed_note(0), "resumed 0 steps");
    }
}
