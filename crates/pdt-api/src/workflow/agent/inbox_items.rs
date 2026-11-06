//! The Inbox side of agent steps: the questions and asking-tool calls a session puts to the user, and the answers to
//! those, stalled, loop-limit, manual and timed-out items.

use super::{R, USER, err, finish_with, prompt_key, queue_message, session_run, waiting_step};
use crate::mcp::tools::Answered;
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
/// which [`answer_confirm`] answers. For a batch call (the PM's `inbox_decide`, PDT-475) `rows` (an array, one per
/// entry of the input's `items`; null otherwise) is the table the user unticks rows of; their Allow runs the call with
/// the rows they kept.
pub(crate) async fn ask_confirm(st: &AppState, s: &Session, tool: &str, input: &Value, rows: Value) -> R<InboxItem> {
    let title = crate::mcp::tools::tool_title(st, tool);
    let agent = s.agent.map(|a| a.label());
    let shown = st.store.redactor().json(input);
    // The item shows a secret's value by its name, and says so (PDT-539): only the waiting call has the value.
    let secret = shown != *input;
    let mut body = json!({"tool": tool, "toolTitle": title, "input": shown, "agent": agent, "sessionTitle": s.title});
    if secret {
        body["secretNote"] = json!(crate::conversations::SECRET_NOTE);
    }
    if rows.is_array() {
        body["rows"] = rows;
    }
    ask(st, s, InboxKind::Confirm, title, body).await
}

/// A batch call's input narrowed to the rows the user kept (`kept`, 0-based; every row when None): its `items`, and
/// the tickets of the rows they unticked. The input as it is when the item has no rows.
fn kept_input(item: &InboxItem, input: Value, kept: Option<&[u32]>) -> R<(Value, Vec<String>)> {
    let Some(rows) = item.body.get("rows").and_then(Value::as_array) else { return Ok((input, vec![])) };
    let keys: Vec<String> =
        rows.iter().map(|r| r.get("ticketKey").and_then(Value::as_str).unwrap_or("?").to_string()).collect();
    keep_rows(input, &keys, kept)
}

/// [`kept_input`] for a call whose rows are the tickets `keys`, one per entry of its input's `items`.
pub(crate) fn keep_rows(input: Value, keys: &[String], kept: Option<&[u32]>) -> R<(Value, Vec<String>)> {
    let keep = pdt_core::inbox_decide::kept(keys.len(), kept)?;
    let mut input = input;
    let items = input.get_mut("items").and_then(Value::as_array_mut).ok_or("the call has no items")?;
    if items.len() != keys.len() {
        return Err("the call's items don't match its rows".into());
    }
    let dropped = (0..keys.len()).filter(|n| !keep.contains(n)).map(|n| keys[n].clone()).collect();
    *items = keep.into_iter().map(|n| items[n].clone()).collect();
    Ok((input, dropped))
}

/// Runs an asking tool's call the user allowed once it no longer waited, as the session's call with their yes
/// (`dropped`: the rows they unticked): what the session is told, and the result.
pub(crate) async fn run_allowed(
    st: &AppState,
    s: &Session,
    tool: &str,
    title: &str,
    input: Value,
    dropped: &[String],
) -> (String, Value) {
    let unticked = if dropped.is_empty() {
        String::new()
    } else {
        format!(" The user unticked {}, so PDT left them as they were.", dropped.join(", "))
    };
    // The persona may have lost the tool since it asked (PDT-501, PDT-539).
    if !crate::mcp::session_tools(st, s).await.iter().any(|t| t == tool) {
        let e = format!("{tool} is no longer one of this session's tools");
        return (
            format!("[PDT] The user allowed your {tool} call ({title}), but PDT didn't run it: {e}."),
            json!({"allowed": true, "error": e}),
        );
    }
    let cx = crate::mcp::tools::Cx { st, session: s, input: &input, confirmed: true };
    let r = st.store.redactor();
    match st.registry().call_tool(&cx, tool, input.clone()).await {
        Ok(v) => {
            let v = r.json(&v);
            (
                format!("[PDT] The user allowed your {tool} call ({title}).{unticked} PDT ran it; its result:\n{v}"),
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
}

/// What a session is told of the user's Deny of a call that no longer waited, with their reason if they gave one.
pub(crate) fn denied_text(tool: &str, title: &str, why: Option<String>) -> String {
    format!("[PDT] The user denied your {tool} call ({title}): don't make it.{}", crate::mcp::tools::said(why))
}

/// The user's Allow or Deny of an asking tool's call (PDT-256): Allow runs it as the session's call with the user's
/// yes; the session hears the result either way.
/// `kept`: the rows a batch call's Allow keeps ([`ask_confirm`]); `why`: the user's reason for a Deny.
pub(crate) async fn answer_confirm(
    st: &AppState,
    item: &InboxItem,
    choice: &str,
    kept: Option<&[u32]>,
    why: Option<&str>,
) -> R<Value> {
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
    // Ended first, so a second click runs nothing.
    let resolution = match (allowed, kept) {
        (true, Some(rows)) if item.body.get("rows").is_some() => json!({"choice": choice, "rows": rows}),
        _ => json!({"choice": choice}),
    };
    // A call still waiting on the item (PDT-492) goes on in the agent's turn with the answer. Its waiter comes out
    // before the item ends, so the wait can't take the ended item for one closed unanswered.
    let waiter = super::confirm_wait::take(st, &item.id);
    if let Err(e) = end_item(st, item, resolution).await {
        if let Some(w) = waiter {
            w.put_back(st, &item.id);
        }
        return Err(e);
    }
    let why = why.map(String::from);
    let waited = if allowed { Answered::Allowed(kept.map(<[u32]>::to_vec)) } else { Answered::Denied(why.clone()) };
    if waiter.is_some_and(|w| w.answer(waited)) {
        let _locked = crate::workflow::engine::lock_run(st, item.run_id.as_deref()).await;
        working_again(st, item).await?;
        return Ok(json!({"allowed": allowed, "waited": true}));
    }
    let (text, out) = if allowed && item.body.get("secretNote").is_some() {
        // The item keeps the call's input with the secret's name in place of its value (PDT-539): running that would
        // send the name, so the agent makes the call again.
        let e = "it held a secret's value, which PDT keeps only while the call waits";
        (
            format!(
                "[PDT] The user allowed your {tool} call ({title}), but PDT didn't run it: {e}. Make the call again."
            ),
            json!({"allowed": true, "error": e}),
        )
    } else if allowed {
        run_allowed(st, &s, tool, title, input, &dropped).await
    } else {
        (denied_text(tool, title, why), json!({"allowed": false}))
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
    let group = if kind == InboxKind::Confirm { InboxGroup::Approval } else { InboxGroup::Question };
    let choices = match kind {
        InboxKind::Confirm => {
            vec![InboxChoice::new(inbox::ALLOW, "Allow", true), InboxChoice::new(inbox::DENY, "Deny", false)]
        }
        _ => vec![],
    };
    let item = crate::inbox::new_item(kind, group, title, body)
        .project(s.project_id.clone())
        .ticket(ticket_id)
        .attempt(s.attempt_id.clone().or_else(|| binding.as_ref().and_then(|a| a.attempt_id.clone())))
        .session(s.id.clone())
        .run(binding.as_ref().map(|a| a.run_id.clone()))
        .step(binding.as_ref().map(|a| a.step_id.clone()))
        .persona(s.persona.as_ref().map(|p| p.id.clone()))
        .choices(choices);
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
