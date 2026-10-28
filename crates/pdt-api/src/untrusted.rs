//! Untrusted input (PDT-201): which sessions read it, the PM's confirmed calls that may let an external ticket in, and
//! the secret tools kept away from it. The Todo gate itself is pdt-core's `external_gate`, applied by the store.

use crate::routes::AppState;
use pdt_core::session::Session;
use pdt_core::ticket::{TicketSource, TicketSummary};
use serde_json::Value;
use std::time::{Duration, Instant};

/// How long the user's yes to a call stays good for that call.
const GRANT_TTL: Duration = Duration::from_secs(10 * 60);

/// The PM's asking-tool calls waiting on the user (`(session, request)` → `(tool, input)`), and the ones the user said
/// yes to, until the call comes in.
#[derive(Default)]
pub(crate) struct Grants {
    pending: std::sync::Mutex<std::collections::HashMap<(String, String), (String, Value)>>,
    granted: std::sync::Mutex<Vec<(String, String, Value, Instant)>>,
}

impl Grants {
    /// A request PDT must not answer for the user: remember it so their yes can be matched to the call.
    pub(crate) fn hold(&self, session_id: &str, request_id: &str, tool: &str, input: &Value) {
        let mut p = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        p.insert((session_id.into(), request_id.into()), (tool.into(), input.clone()));
    }

    /// The user answered a request in the UI: a yes to a held one is good for one call with that input.
    pub(crate) fn answered(&self, session_id: &str, request_id: &str, approved: bool) {
        let held =
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&(session_id.into(), request_id.into()));
        if let (true, Some((tool, input))) = (approved, held) {
            let mut g = self.granted.lock().unwrap_or_else(|e| e.into_inner());
            g.retain(|x| x.3.elapsed() < GRANT_TTL);
            g.push((session_id.into(), tool, input, Instant::now()));
        }
    }

    /// Takes the user's yes to this call, if they gave one.
    pub(crate) fn take(&self, session_id: &str, tool: &str, input: &Value) -> bool {
        let mut g = self.granted.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|x| x.3.elapsed() < GRANT_TTL);
        match g.iter().position(|x| x.0 == session_id && x.1 == tool && same_call(&x.2, input)) {
            Some(i) => {
                g.remove(i);
                true
            }
            None => false,
        }
    }
}

/// The same call: equal arguments, leaving out ones given as null.
fn same_call(a: &Value, b: &Value) -> bool {
    let clean = |v: &Value| match v {
        Value::Object(m) => {
            Value::Object(m.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        other => other.clone(),
    };
    clean(a) == clean(b)
}

/// An external ticket the user hasn't let in yet.
pub(crate) fn held_back(t: &TicketSummary) -> bool {
    t.source == TicketSource::External && !t.admitted
}

/// Whether an edit would let an external ticket in (PDT-201's Todo gate).
pub(crate) fn lets_in(t: &TicketSummary, p: &pdt_core::ticket::ValidPatch) -> bool {
    held_back(t) && pdt_core::ticket::boards(Some(t.stage), p.stage, &t.labels, p.labels.as_deref())
}

/// Whether a pdt-mcp call (`tool`, `input`) from a session is one PDT never answers for the user (PDT-201): the PM's
/// `edit_ticket` letting an external ticket in.
pub(crate) async fn needs_the_user(st: &AppState, s: &Session, tool: &str, input: &Value) -> bool {
    #[derive(serde::Deserialize)]
    struct Edit {
        ticket: String,
        #[serde(default)]
        stage: Option<pdt_core::ticket::TicketStage>,
        #[serde(default)]
        labels: Option<Vec<String>>,
    }
    if tool != "edit_ticket" {
        return false;
    }
    let Ok(e) = serde_json::from_value::<Edit>(input.clone()) else { return false };
    let Ok(id) = crate::mcp::tools::resolve_ticket(st, &e.ticket, Some(&s.project_id)).await else { return false };
    let Ok(Some(t)) = st.store.get_ticket_summary(&id).await else { return false };
    let labels = match e.labels.as_deref().map(pdt_core::ticket::normalize_labels) {
        Some(Ok(l)) => Some(l),
        Some(Err(_)) => return false,
        None => None,
    };
    lets_in(&t, &pdt_core::ticket::ValidPatch { stage: e.stage, labels, ..Default::default() })
}

/// Whether a session reads untrusted input: it runs a triage persona, or is an agent step of a workflow run that
/// outside input started (a new issue, a webhook).
pub(crate) async fn triaging(st: &AppState, s: &Session) -> bool {
    let Some(p) = &s.persona else { return false };
    if let Ok(Some(persona)) = st.store.get_persona(&p.id).await
        && pdt_core::persona::locked_down(&persona.role)
    {
        return true;
    }
    let Some(run) = crate::workflow::agent::session_run(st, &s.id).await else { return false };
    let Ok(Some(run)) = st.store.get_workflow_run(&run).await else { return false };
    untrusted_run(st, &run).await
}

/// Whether a workflow run was started by outside input.
pub(crate) async fn untrusted_run(st: &AppState, run: &pdt_core::workflow::api::Run) -> bool {
    let Some(event) = &run.trigger_event_id else { return false };
    matches!(st.store.get_log_event(event).await, Ok(Some(e)) if pdt_core::workflow::events::untrusted(&e.kind))
}

/// The secret tools a session is refused (PDT-201): one that triages, or whose ticket is external and not let in yet.
pub(crate) async fn refuse_secrets(st: &AppState, s: &Session) -> Result<(), String> {
    if triaging(st, s).await {
        return Err("this session reads input from outside PDT, so it gets no secrets".into());
    }
    if let Some(id) = crate::mcp::tools::session_ticket(st, s).await
        && let Ok(Some(t)) = st.store.get_ticket_summary(&id).await
        && held_back(&t)
    {
        return Err(format!(
            "{} came from outside PDT and the user hasn't moved it to To do yet, so its sessions get no secrets",
            t.key
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_yes_is_good_for_one_matching_call() {
        let g = Grants::default();
        let input = json!({"ticket": "P-1", "stage": "todo"});
        g.hold("s", "r1", "edit_ticket", &input);
        // Nothing until the user answers.
        assert!(!g.take("s", "edit_ticket", &input));
        g.answered("s", "r1", true);
        assert!(!g.take("other", "edit_ticket", &input), "another session's");
        assert!(!g.take("s", "edit_ticket", &json!({"ticket": "P-2", "stage": "todo"})), "another call");
        assert!(g.take("s", "edit_ticket", &json!({"ticket": "P-1", "stage": "todo", "title": null})));
        assert!(!g.take("s", "edit_ticket", &input), "used up");
        // A no grants nothing; an answer to a request PDT didn't hold neither.
        g.hold("s", "r2", "edit_ticket", &input);
        g.answered("s", "r2", false);
        g.answered("s", "r3", true);
        assert!(!g.take("s", "edit_ticket", &input));
    }
}
