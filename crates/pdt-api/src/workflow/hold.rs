//! A ticket's merge and move to Done wait while an agent's question to the user about it is open (PDT-628): every
//! `ask_user` question is blocking unless the agent said otherwise (pdt-core `InboxItem::holds_ticket`). The nodes that
//! merge or finish (`attempt.merge`, `ticket.finish`, `ticket.move` to Done, `ticket.close_parent`) wait, looking
//! again every [`LOOK_MS`] and at once when a question is answered ([`wake`]); a workflow's or an agent's merge is
//! refused (`review::merge_attempt`). The user's own merge or move is theirs to make.

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::Outcome;
use pdt_core::inbox::{InboxItem, held_reason};
use serde_json::json;

/// How often a held step looks again (an answer wakes it at once).
const LOOK_MS: i64 = 60_000;

/// What a held step's `wait.hold` says.
const HOLD: &str = "question";

/// The open questions holding `tickets`, oldest first.
pub(crate) async fn holding(st: &AppState, tickets: &[&str]) -> Result<Vec<InboxItem>, String> {
    st.store.questions_holding(tickets).await.map_err(crate::workflow::err)
}

/// Why `tickets` can't merge or move to Done yet, when an open question holds one of them.
pub(crate) async fn held(st: &AppState, tickets: &[&str]) -> Result<Option<String>, String> {
    let items = holding(st, tickets).await?;
    let Some(first) = items.first() else { return Ok(None) };
    let id = first.ticket_id.as_deref().unwrap_or_default();
    let key = match st.store.get_ticket_summary(id).await.map_err(crate::workflow::err)? {
        Some(t) => t.key,
        None => "The ticket".into(),
    };
    Ok(Some(held_reason(&key, &items.iter().collect::<Vec<_>>())))
}

/// A step's wait while a question holds `tickets`: it runs again once they are answered. None when nothing holds.
pub(crate) async fn wait_for_answers(st: &AppState, tickets: &[&str]) -> Result<Option<Outcome>, String> {
    let Some(why) = held(st, tickets).await? else { return Ok(None) };
    Ok(Some(Outcome::Wait {
        wait: json!({"retry": true, (crate::workflow::engine::POLL): true, "hold": HOLD, "waiting": why}),
        until: Some(now_ms() + LOOK_MS),
        timeout_port: None,
        output: json!({"held": why}),
    }))
}

/// A question was answered (or closed): the steps it may have held look again now, and a merge PDT holds for CI
/// (`merge_requested`) is tried again for `ticket`. Spawned: the caller may hold the run's lock.
pub(crate) fn wake(st: &AppState, ticket: Option<String>) {
    let st = st.clone();
    tokio::spawn(async move {
        let steps = match st.store.steps_held_for_questions().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "could not read the steps a question holds");
                return;
            }
        };
        for step in steps {
            let _locked = crate::workflow::engine::lock_run(&st, Some(&step.run_id)).await;
            let Ok(Some(mut s)) = st.store.get_step(&step.id).await else { continue };
            if s.status != pdt_core::workflow::api::StepStatus::Waiting || s.wait.get("hold").is_none() {
                continue;
            }
            s.wake_at = Some(now_ms());
            if let Err(e) = st.store.put_step(&s).await {
                tracing::warn!(step = %s.id, error = %e, "could not wake a step a question held");
                continue;
            }
            if let Err(e) = st.store.wake_workflow_run(&s.run_id, now_ms()).await {
                tracing::warn!(run = %s.run_id, error = %e, "could not wake a run a question held");
            }
        }
        // A merge PDT holds until CI passes was refused while the question was open: CI has passed by now, or its
        // pass merges it.
        let Some(ticket) = ticket else { return };
        let Ok(Some(info)) = st.store.active_attempt(&ticket).await else { return };
        let held_by_pdt = crate::github::watched_pr(&st, &info.attempt.id).is_some_and(|p| p.merge_held);
        if info.merge_requested && held_by_pdt && holding(&st, &[&ticket]).await.is_ok_and(|h| h.is_empty()) {
            crate::review::merge_held(st.clone(), info.attempt.id.clone()).await;
        }
    });
}
