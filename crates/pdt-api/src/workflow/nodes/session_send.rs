//! `session.send`: a message into an agent session, with the PR notices' delivery rule (Phase 7a spec §5.2). The
//! step queues it in the outbox and waits; `workflow::outbox` delivers it and resumes the run.

use crate::routes::now_ms;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_store::SessionMessage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TimelineRecord {
    /// Nothing on the ticket's timeline (the run shows what happened).
    #[default]
    None,
    /// A `pr` event with the trigger's `notice` and whether the agent was told (today's PR notices).
    Pr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SessionSendConfig {
    /// To the attempt's newest running agent session (`{{trigger.attemptId}}`)…
    #[serde(default)]
    pub attempt: Option<String>,
    /// …or to this session.
    #[serde(default)]
    pub session: Option<String>,
    #[schemars(extend("x-format" = "markdown"))]
    pub text: String,
    /// A key already sent to the attempt is not sent again (`{{trigger.notice.key}}`).
    #[serde(default)]
    pub dedupe: Option<String>,
    #[serde(default)]
    pub timeline: TimelineRecord,
    /// A conversation that is not running (stopped, or interrupted by a restart) reopens to take it, as it would for
    /// the user's own message: for what the user wrote (their ticket comment). Otherwise only a running session is
    /// told (PDT-144).
    #[serde(default)]
    pub reopen: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionSendOutput {
    pub told: bool,
    pub session_id: Option<String>,
}

pub struct SessionSend;

impl NodeKind for SessionSend {
    fn spec(&self) -> KindSpec {
        spec::<SessionSendConfig, SessionSendOutput>(
            "session.send",
            1,
            Category::Action,
            "Tell the agent",
            "Types a message into an agent session once it is idle and nobody is typing (a conversation: no turn \
             running). `untold` when the attempt has no running agent session (with `reopen`, nor a conversation to reopen); \
             `skipped` when the dedupe key was sent, or when the trigger is what someone who isn't a collaborator \
             on the repository wrote (that is never sent into a session).",
            &[("sent", "Told"), ("untold", "No session to tell"), ("skipped", "Already sent")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: SessionSendConfig = parse(config)?;
        if c.attempt.is_none() == c.session.is_none() {
            return Err("give either attempt or session".into());
        }
        if c.text.trim().is_empty() {
            return Err("text is empty".into());
        }
        Ok(())
    }

    fn agent_text(&self) -> &'static [&'static str] {
        &["text"]
    }

    /// Its message may be delivered at once: the step is waiting by then.
    fn in_engine_lock(&self, _category: Category) -> bool {
        true
    }

    fn default_timeout_ms(&self) -> u64 {
        30_000
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: SessionSendConfig = parse(&input)?;
            // What an outsider wrote stays in PDT (PDT-202): the ticket's timeline shows it.
            if let Some(why) = crate::untrusted::outsider_refusal(cx.ctx, cx.origin) {
                tracing::info!(run = %cx.run.id, "a message an outsider wrote was not sent to the agent");
                return Ok(Outcome::next("skipped", json!({"quiet": true, "reason": why})));
            }
            let attempt = c.attempt.filter(|a| !a.trim().is_empty());
            let session = c.session.filter(|s| !s.trim().is_empty());
            if attempt.is_none() && session.is_none() {
                return Err("no attempt or session to send to".into());
            }
            let store = &cx.st.store;
            let ticket_id = match (&attempt, cx.run.ticket_id.clone()) {
                (_, Some(t)) => Some(t),
                (Some(a), None) => store.get_attempt(a).await.map_err(crate::workflow::err)?.map(|i| i.ticket_id),
                _ => None,
            };
            let dedupe = c.dedupe.filter(|d| !d.trim().is_empty());
            if let (Some(a), Some(d)) = (&attempt, &dedupe) {
                let mut sent = store.session_message_sent(a, d).await.map_err(crate::workflow::err)?;
                // Notices told before the event log existed are on the timeline.
                if !sent && let Some(t) = &ticket_id {
                    sent = store.pr_notice_keys(t, a).await.map_err(crate::workflow::err)?.contains(d);
                }
                if sent {
                    return Ok(Outcome::next("skipped", json!({"quiet": true, "reason": "already sent"})));
                }
            }
            if cx.dry_run {
                let to = match (&attempt, &session) {
                    (_, Some(s)) => format!("session {s}"),
                    (Some(a), _) => format!("the agent session of attempt {a}"),
                    _ => String::new(),
                };
                return Ok(Outcome::next("sent", json!({"would": format!("tell {to}: {}", c.text)})));
            }
            let timeline = match c.timeline {
                TimelineRecord::None => Value::Null,
                TimelineRecord::Pr => {
                    let n = cx.ctx.pointer("/trigger/notice").cloned().unwrap_or(Value::Null);
                    json!({"kind": "pr", "data": {
                        "attemptId": attempt, "notice": n.get("kind"), "key": n.get("key").cloned().or(dedupe.clone().map(Value::from)),
                        "pr": n.get("pr"), "url": n.get("url"), "jobs": n.get("jobs").cloned().unwrap_or(json!([])),
                        "text": c.text, "workflowId": cx.workflow.id, "workflowName": cx.workflow.name, "runId": cx.run.id,
                    }})
                }
            };
            let m = SessionMessage {
                id: uuid::Uuid::now_v7().to_string(),
                attempt_id: attempt,
                session_id: session,
                ticket_id,
                dedupe,
                text: c.text,
                timeline,
                run_id: Some(cx.run.id.clone()),
                step_id: Some(cx.step.id.clone()),
                status: "waiting".into(),
                told_session: None,
                created_at: now_ms(),
                resolved_at: None,
                by_user: false,
                pending_id: None,
                reopens: c.reopen,
            };
            let m = store.queue_session_message(&m).await.map_err(crate::workflow::err)?;
            // Still waiting, or on its way to a conversation (told once the agent echoes it): wait for it.
            if m.status != "waiting" && m.status != "sending" {
                // A re-run step whose message was already delivered.
                let told = m.status == "told";
                let port = if told { "sent" } else { "untold" };
                return Ok(Outcome::next(port, SessionSendOutput { told, session_id: m.told_session }));
            }
            Ok(Outcome::Wait { wait: json!({"delivery": m.id}), until: None, timeout_port: None, output: Value::Null })
        })
    }
}
