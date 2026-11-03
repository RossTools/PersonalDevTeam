//! The Inbox side of agent steps: the questions and asking-tool calls a session puts to the user, and the answers to
//! those, stalled, loop-limit, manual and timed-out items.

use super::{Confirmed, R, USER, err, finish_with, prompt_key, queue_message, session_run, waiting_step};
use crate::routes::{AppState, now_ms};
use pdt_core::conversation::SessionMode;
use pdt_core::inbox::{self, InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus, Question};
use pdt_core::session::{Session, SessionStatus};
use pdt_core::workflow::agent::{AgentStepState, OutcomeSpec, STOPPED, nudge_message, resumable_step};
use pdt_core::workflow::api::StepStatus;
use serde_json::{Value, json};

/// `ask_user` (pdt-mcp): an Inbox question; a step waits for the answers instead of stalling.
pub(crate) async fn ask_user(st: &AppState, s: &Session, questions: Vec<Question>) -> R<Value> {
    let questions = inbox::valid_questions(questions)?;
    let first = questions[0].question.clone();
    let item = ask(st, s, InboxKind::Question, first, json!({"questions": questions})).await?;
    Ok(json!({"asked": true, "inboxItemId": item.id,
              "message": "Your questions are in the user's Inbox. Their answers will come as a message; end your turn now."}))
}

/// An asking tool's call from an agent that can't ask in its own prompt (PDT-256): an Inbox item with Allow and Deny,
/// which [`answer_confirm`] answers.
pub(crate) async fn ask_confirm(st: &AppState, s: &Session, tool: &str, input: &Value) -> R<InboxItem> {
    ask_confirm_rows(st, s, tool, input, Value::Null).await
}

/// [`ask_confirm`] for a batch call (the PM's `inbox_decide`, PDT-475): `rows` (an array, one per entry of the
/// input's `items`) is the table the user unticks rows of; their Allow runs the call with the rows they kept.
pub(crate) async fn ask_confirm_rows(
    st: &AppState,
    s: &Session,
    tool: &str,
    input: &Value,
    rows: Value,
) -> R<InboxItem> {
    let info = st.registry().tool_infos().into_iter().find(|t| t.name == tool);
    let title = info.map(|t| t.title).filter(|t| !t.is_empty()).unwrap_or_else(|| tool.to_string());
    let agent = s.agent.map(|a| a.label());
    let mut body = json!({"tool": tool, "toolTitle": title, "input": st.store.redactor().json(input), "agent": agent,
                          "sessionTitle": s.title});
    if rows.is_array() {
        body["rows"] = rows;
    }
    ask(st, s, InboxKind::Confirm, title, body).await
}

/// A batch call's input narrowed to the rows the user kept (`kept`, 0-based; every row when None): its `items`, and
/// the tickets of the rows they unticked. The input as it is when the item has no rows.
fn kept_input(item: &InboxItem, input: Value, kept: Option<&[u32]>) -> R<(Value, Vec<String>)> {
    let Some(rows) = item.body.get("rows").and_then(Value::as_array) else { return Ok((input, vec![])) };
    let keep = pdt_core::inbox_decide::kept(rows.len(), kept)?;
    let mut input = input;
    let items = input.get_mut("items").and_then(Value::as_array_mut).ok_or("the call has no items")?;
    if items.len() != rows.len() {
        return Err("the call's items don't match its rows".into());
    }
    let dropped = (0..rows.len())
        .filter(|n| !keep.contains(n))
        .map(|n| rows[n].get("ticketKey").and_then(Value::as_str).unwrap_or("?").to_string())
        .collect();
    *items = keep.into_iter().map(|n| items[n].clone()).collect();
    Ok((input, dropped))
}

/// The user's Allow or Deny of an asking tool's call (PDT-256): Allow runs it as the session's call with the user's
/// yes; the session hears the result either way.
/// `kept`: the rows a batch call's Allow keeps ([`ask_confirm_rows`]).
pub(crate) async fn answer_confirm(st: &AppState, item: &InboxItem, choice: &str, kept: Option<&[u32]>) -> R<Value> {
    let s = asker(st, item).await?;
    let tool = item.body.get("tool").and_then(Value::as_str).ok_or("the item names no tool")?;
    let title = item.body.get("toolTitle").and_then(Value::as_str).unwrap_or(tool);
    let allowed = match choice {
        inbox::ALLOW => true,
        inbox::DENY => false,
        other => return Err(format!("{other} is not a choice here")),
    };
    // The rows are checked before the item is answered.
    let input = item.body.get("input").cloned().unwrap_or_else(|| json!({}));
    let (input, dropped) = if allowed { kept_input(item, input, kept)? } else { (input, vec![]) };
    let unticked = if dropped.is_empty() {
        String::new()
    } else {
        format!(" The user unticked {}, so PDT left them as they were.", dropped.join(", "))
    };
    // Ended first, so a second click runs nothing.
    let resolution = match (allowed, kept) {
        (true, Some(rows)) if item.body.get("rows").is_some() => json!({"choice": choice, "rows": rows}),
        _ => json!({"choice": choice}),
    };
    end_item(st, item, resolution).await?;
    // The call still waits on the item (PDT-492): it goes on in the agent's turn with the answer.
    let waited = if allowed { Confirmed::Allowed(kept.map(<[u32]>::to_vec)) } else { Confirmed::Denied };
    if super::confirm_wait::answer(st, &item.id, waited) {
        let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
        working_again(st, item).await?;
        return Ok(json!({"allowed": allowed, "waited": true}));
    }
    let (text, out) = if allowed {
        let cx = crate::mcp::tools::Cx { st, session: &s, input: &input, confirmed: true };
        let r = st.store.redactor();
        match st.registry().call_tool(&cx, tool, input.clone()).await {
            Ok(v) => {
                let v = r.json(&v);
                (
                    format!(
                        "[PDT] The user allowed your {tool} call ({title}).{unticked} PDT ran it; its result:\n{v}"
                    ),
                    json!({"allowed": true, "result": v}),
                )
            }
            Err(e) => {
                let e = r.string(&e);
                (
                    format!("[PDT] The user allowed your {tool} call ({title}), but it failed: {e}{unticked}"),
                    json!({"allowed": true, "error": e}),
                )
            }
        }
    } else {
        (format!("[PDT] The user denied your {tool} call ({title}): don't make it."), json!({"allowed": false}))
    };
    let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
    tell(st, item, &s, &text).await?;
    Ok(out)
}

/// Opens an item a session waits on the user for (a question, a secret request, an asking tool's call), replacing its
/// step's open one of that kind; the step is asking until it is answered.
pub(crate) async fn ask(st: &AppState, s: &Session, kind: InboxKind, title: String, body: Value) -> R<InboxItem> {
    let run = session_run(st, &s.id).await;
    let _locked = crate::workflow::engine::lock_run(st, run.as_deref()).await;
    let binding = st.store.active_agent_step(&s.id).await.map_err(err)?;
    if let Some(a) = &binding {
        // A new question replaces the step's open one.
        for old in st.store.open_items_for_step(&a.step_id).await.map_err(err)? {
            if old.kind == kind {
                st.store
                    .end_inbox_item(&old.id, InboxStatus::Closed, &json!({"replaced": true}), now_ms())
                    .await
                    .map_err(err)?;
            }
        }
    }
    let ticket_id = crate::mcp::tools::session_ticket(st, s).await;
    let item = InboxItem {
        id: uuid::Uuid::now_v7().to_string(),
        kind,
        group: if kind == InboxKind::Confirm { InboxGroup::Approval } else { InboxGroup::Question },
        status: InboxStatus::Open,
        project_id: Some(s.project_id.clone()),
        ticket_id,
        ticket_key: None,
        ticket_title: None,
        attempt_id: s.attempt_id.clone().or_else(|| binding.as_ref().and_then(|a| a.attempt_id.clone())),
        session_id: Some(s.id.clone()),
        run_id: binding.as_ref().map(|a| a.run_id.clone()),
        step_id: binding.as_ref().map(|a| a.step_id.clone()),
        persona_id: s.persona.as_ref().map(|p| p.id.clone()),
        persona_name: None,
        change_request: None,
        title,
        body,
        choices: match kind {
            InboxKind::Confirm => {
                vec![InboxChoice::new(inbox::ALLOW, "Allow", true), InboxChoice::new(inbox::DENY, "Deny", false)]
            }
            _ => vec![],
        },
        resolution: Value::Null,
        created_at: now_ms(),
        resolved_at: None,
    };
    let item = st.store.open_inbox_item(&item).await.map_err(err)?;
    if let Some(mut a) = binding {
        a.state = AgentStepState::Asking;
        a.updated_at = now_ms();
        st.store.put_agent_step(&a).await.map_err(err)?;
    }
    Ok(item)
}

/// Answers a stalled or loop-limit item, or a question (from the Inbox).
pub(crate) async fn answer_item(st: &AppState, item: &InboxItem, answer: &inbox::InboxAnswer) -> R<Value> {
    match item.kind {
        InboxKind::Question => answer_question(st, item, answer).await,
        InboxKind::Secret => crate::vault::answer_request(st, item, answer).await,
        InboxKind::Stalled => {
            let choice = answer.choice.as_deref().ok_or("choose Nudge or an outcome")?;
            let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
            let step_id = item.step_id.as_deref().ok_or("the item has no step")?;
            let mut a = st.store.get_agent_step(step_id).await.map_err(err)?.ok_or("the step is gone")?;
            let step = waiting_step(st, &a).await?;
            if choice == inbox::NUDGE || choice == inbox::RESEND {
                let s = st
                    .store
                    .get_session(a.session_id.as_deref().unwrap_or_default())
                    .await
                    .map_err(err)?
                    .ok_or("the step's session is gone: take it over instead")?;
                if s.status != SessionStatus::Running && s.mode != SessionMode::Conversation {
                    return Err("the step's terminal session has stopped: restart it, or take the step over".into());
                }
                if choice == inbox::RESEND {
                    // The step's undelivered first message, again, as the user's (PDT-144).
                    let text = item.body.get("message").and_then(Value::as_str).ok_or("the item has no message")?;
                    queue_message(st, &s, text, Some(prompt_key(&a.step_id)), a.ticket_id.clone(), USER).await?;
                } else {
                    let text = answer
                        .text
                        .as_deref()
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| nudge_message(&a.outcomes));
                    queue_message(st, &s, &text, None, a.ticket_id.clone(), USER).await?;
                    a.nudges += 1;
                }
                a.state = AgentStepState::Working;
                // Its wait on background tasks starts over if the nudged turn ends with some still running.
                a.background_since = None;
                a.background.clear();
                a.updated_at = now_ms();
                st.store.put_agent_step(&a).await.map_err(err)?;
                end_item(st, item, json!({"choice": choice})).await?;
                return Ok(if choice == inbox::RESEND { json!({"resent": true}) } else { json!({"nudged": true}) });
            }
            let outcome = inbox::choice_outcome(choice).ok_or_else(|| format!("{choice} is not a choice here"))?;
            if !a.outcomes.iter().any(|o| o.id == outcome) {
                return Err(format!("{outcome} is not an outcome of this step"));
            }
            let summary = answer
                .text
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "Taken over by the user.".into());
            end_item(st, item, json!({"choice": choice})).await?;
            finish_with(st, a, &step, outcome, summary, true).await?;
            Ok(json!({"outcome": outcome}))
        }
        InboxKind::LoopLimit => {
            let choice = answer.choice.as_deref().ok_or("choose Once more, an outcome or Stop")?;
            let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
            let step_id = item.step_id.as_deref().ok_or("the item has no step")?;
            let mut step = st.store.get_step(step_id).await.map_err(err)?.ok_or("the step is gone")?;
            if step.status != StepStatus::Waiting {
                return Err("this workflow step has already ended".into());
            }
            end_item(st, item, json!({"choice": choice})).await?;
            if choice == inbox::AGAIN {
                step.status = StepStatus::Pending;
                step.wait = json!({"loopAllowed": true});
                step.wake_at = None;
                st.store.put_step(&step).await.map_err(err)?;
                st.store.wake_workflow_run(&step.run_id, now_ms()).await.map_err(err)?;
                return Ok(json!({"again": true}));
            }
            if choice == inbox::STOP {
                crate::workflow::engine::complete_step(st, &step, STOPPED, json!({"stopped": true, "by": "user"}))
                    .await?;
                return Ok(json!({"stopped": true}));
            }
            let outcome = inbox::choice_outcome(choice).ok_or_else(|| format!("{choice} is not a choice here"))?;
            let outcomes: Vec<OutcomeSpec> =
                serde_json::from_value(item.body.get("outcomes").cloned().unwrap_or_default()).unwrap_or_default();
            if !outcomes.iter().any(|o| o.id == outcome) {
                return Err(format!("{outcome} is not an outcome of this step"));
            }
            crate::workflow::engine::complete_step(st, &step, outcome, json!({"outcome": outcome, "by": "user"}))
                .await?;
            Ok(json!({"outcome": outcome}))
        }
        InboxKind::Gate => {
            let choice = answer.choice.as_deref().ok_or("choose one of the gate's choices")?;
            let (run, step) = (item.run_id.as_deref().unwrap_or_default(), item.step_id.as_deref().unwrap_or_default());
            crate::workflow::engine::answer(st, run, step, choice, answer.text.as_deref()).await?;
            Ok(json!({"choice": choice}))
        }
        InboxKind::Manual => {
            let choice = answer.choice.as_deref().ok_or("choose the step's outcome")?;
            let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
            let step_id = item.step_id.as_deref().ok_or("the item has no step")?;
            let a = st.store.get_agent_step(step_id).await.map_err(err)?.ok_or("the step is gone")?;
            let step = waiting_step(st, &a).await?;
            let outcome = inbox::choice_outcome(choice).ok_or_else(|| format!("{choice} is not a choice here"))?;
            if !a.outcomes.iter().any(|o| o.id == outcome) {
                return Err(format!("{outcome} is not an outcome of this step"));
            }
            let summary =
                answer.text.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "Done by the user.".into());
            end_item(st, item, json!({"choice": choice})).await?;
            finish_with(st, a, &step, outcome, summary, true).await?;
            Ok(json!({"outcome": outcome}))
        }
        InboxKind::TimedOut => {
            let choice = answer.choice.as_deref().ok_or("choose Resume or Stop")?;
            let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
            let run_id = item.run_id.as_deref().ok_or("the item has no run")?;
            let steps = st.store.list_steps(run_id).await.map_err(err)?;
            if resumable_step(&steps).map(|s| s.id.as_str()) != item.step_id.as_deref() {
                // Resumed from the run, or it went on otherwise: nothing left to decide.
                end_item(st, item, json!({"choice": choice, "movedOn": true})).await?;
                return Err("the run has already moved on from this step".into());
            }
            if choice == inbox::RESUME {
                crate::workflow::engine::resume_locked(st, run_id, answer.text.clone()).await?;
                return Ok(json!({"resumed": true}));
            }
            if choice == inbox::STOP {
                end_item(st, item, json!({"choice": choice})).await?;
                let run = st.store.get_workflow_run(run_id).await.map_err(err)?;
                if run.is_some_and(|r| !r.status.is_finished()) {
                    crate::workflow::engine::cancel_locked(st, run_id).await?;
                }
                return Ok(json!({"stopped": true}));
            }
            Err(format!("{choice} is not a choice here"))
        }
        InboxKind::Approval => Err("answer an approval in its session".into()),
        InboxKind::Session => Err("answer the session in its tab".into()),
        InboxKind::Notice => Err("a notice waits on no answer: dismiss it".into()),
        InboxKind::Connect => Err("connect GitHub in Settings › Plugins › GitHub".into()),
        InboxKind::McpSignIn => Err("sign in to the MCP server again in Settings › MCP servers".into()),
        // Answered by `inbox::answer`, which knows whether the user's own click sent it.
        InboxKind::Admit => Err("approve or reject a triaged ticket from PDT's Inbox".into()),
        InboxKind::Confirm => Err("allow or deny an agent's call from PDT's Inbox".into()),
        InboxKind::Proposal => Err("accept or discard a workflow proposal from PDT's Inbox".into()),
        InboxKind::TicketProposal => Err("decide a ticket proposal from PDT's Inbox".into()),
        InboxKind::Storage => Err("PDT can't save changes: it waits on no answer, and goes once saving works".into()),
        InboxKind::LowDisk => Err("low disk space waits on no answer: it goes once there is space again".into()),
    }
}

async fn end_item(st: &AppState, item: &InboxItem, resolution: Value) -> R<()> {
    if !st.store.end_inbox_item(&item.id, InboxStatus::Resolved, &resolution, now_ms()).await.map_err(err)? {
        return Err("this item was already answered".into());
    }
    Ok(())
}

/// Sends the user's answers to the asking session; its step works again.
async fn answer_question(st: &AppState, item: &InboxItem, answer: &inbox::InboxAnswer) -> R<Value> {
    let questions: Vec<Question> =
        serde_json::from_value(item.body.get("questions").cloned().unwrap_or_default()).unwrap_or_default();
    let text = inbox::answers_message(&questions, answer)?;
    reply(st, item, &text, json!({"answers": answer.answers, "text": answer.text})).await?;
    Ok(json!({"answered": true}))
}

/// Ends an item the asking session waits on with `resolution`, and sends it `text`; its step works again.
pub(crate) async fn reply(st: &AppState, item: &InboxItem, text: &str, resolution: Value) -> R<()> {
    let s = asker(st, item).await?;
    let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
    end_item(st, item, resolution).await?;
    tell(st, item, &s, text).await
}

/// Sends the asking session `text` about its ended item; its step works again. The caller holds the run's lock.
async fn tell(st: &AppState, item: &InboxItem, s: &Session, text: &str) -> R<()> {
    queue_message(st, s, text, None, item.ticket_id.clone(), USER).await?;
    working_again(st, item).await
}

/// The step that asked with `item` works again, now it is answered. The caller holds the run's lock.
async fn working_again(st: &AppState, item: &InboxItem) -> R<()> {
    if let Some(step) = &item.step_id
        && let Ok(Some(mut a)) = st.store.get_agent_step(step).await
        && a.state == AgentStepState::Asking
    {
        a.state = AgentStepState::Working;
        a.updated_at = now_ms();
        st.store.put_agent_step(&a).await.map_err(err)?;
    }
    Ok(())
}

/// The session an item asks for, while it can still be answered.
pub(crate) async fn asker(st: &AppState, item: &InboxItem) -> R<Session> {
    let sid = item.session_id.as_deref().ok_or("the question has no session to answer")?;
    let s = st.store.get_session(sid).await.map_err(err)?.ok_or("the asking session is gone")?;
    if s.mode != SessionMode::Conversation && s.status != SessionStatus::Running {
        return Err("the asking terminal session has stopped; restart it first".into());
    }
    Ok(s)
}
