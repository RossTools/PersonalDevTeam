//! pdt-mcp's tools (Phase 7b spec §2.2). Inputs are the API's request types where one exists (their JSON Schema from
//! schemars), and each tool runs the function the matching route runs, so the API and the tools can't drift.

use crate::error::ApiError;
use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{Category, catalog};
use axum::Json;
use axum::extract::State;
use pdt_core::inbox::Question;
use pdt_core::mcp::{ToolInfo, input_schema};
use pdt_core::review::{CommentSide, NewReviewComment};
use pdt_core::session::Session;
use pdt_core::ticket::{Actor, valid_comment};
use pdt_core::workflow::agent::OutcomeComment;
use pdt_core::workflow::api::{Author, Scope};
use pdt_core::workflow::doc::WorkflowDoc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// What a tool call knows: PDT and the calling session.
pub(crate) struct Cx<'a> {
    pub st: &'a AppState,
    pub session: &'a Session,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetTicket {
    /// A ticket key (`PDT-12`) or id. Leave it out for this session's ticket.
    #[serde(default)]
    ticket: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SubmitOutcome {
    /// One of the outcomes this step allows.
    pub outcome: String,
    /// What you did and found, in a few sentences: the next step (or the user) reads it.
    pub summary: String,
    /// Line comments on the attempt's changes (a review's findings).
    #[serde(default)]
    pub comments: Vec<OutcomeComment>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AskUser {
    /// Short, concrete questions; give choices where a few answers are likely.
    questions: Vec<Question>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddTicketComment {
    /// Markdown. It is recorded as the agent's (`NewComment` without `by`: an agent never writes as the user).
    body: String,
    /// A ticket key or id; this session's ticket when left out.
    #[serde(default)]
    ticket: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTicket {
    /// A new title (one line).
    #[serde(default)]
    title: Option<String>,
    /// The new description (Markdown), replacing the old one: keep what still holds.
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogSearch {
    /// Words to look for in the kinds' ids, titles and descriptions; every kind when left out.
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    category: Option<Category>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CatalogGet {
    /// The kind's id (`session.send`).
    id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowGet {
    #[serde(default)]
    id: Option<String>,
    /// The workflow's name, when you don't know its id.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowSave {
    /// The workflow to save a new version of; a new (disabled) workflow when left out.
    #[serde(default)]
    id: Option<String>,
    doc: WorkflowDoc,
    /// What changed, for the version history.
    #[serde(default)]
    note: String,
    /// A new workflow's scope (default: the session's project).
    #[serde(default)]
    scope: Option<Scope>,
    #[serde(default)]
    project_id: Option<String>,
}

struct Def {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

/// Tools that change nothing.
const READ_ONLY: [&str; 6] =
    ["get_ticket", "catalog_search", "catalog_get", "workflow_get", "workflow_validate", "workflow_dry_run"];

fn schema<T: JsonSchema>() -> Value {
    input_schema(crate::workflow::catalog::schema_of::<T>())
}

/// Every tool, in `ALL_TOOLS` order.
fn defs() -> Vec<Def> {
    vec![
        Def {
            name: "get_ticket",
            title: "Get the ticket",
            description: "The ticket this session works on (or another, by key): description, stage, links, attempts, \
                          comments, the outcomes earlier agent steps submitted, and, in a workflow step, what the step \
                          allows.",
            schema: schema::<GetTicket>,
        },
        Def {
            name: "submit_outcome",
            title: "Submit the step's outcome",
            description: "Ends this session's workflow step with one of the outcomes it allows; PDT moves the work on \
                          from it. Give a short summary, and line comments for a review.",
            schema: schema::<SubmitOutcome>,
        },
        Def {
            name: "ask_user",
            title: "Ask the user",
            description: "Puts questions in the user's Inbox. Their answers come back as a message; end your turn \
                          after asking. In a workflow step the step waits for them instead of stalling.",
            schema: schema::<AskUser>,
        },
        Def {
            name: "add_ticket_comment",
            title: "Comment on the ticket",
            description: "Adds a comment to the ticket's timeline (this session's ticket unless you name one).",
            schema: schema::<AddTicketComment>,
        },
        Def {
            name: "add_review_comment",
            title: "Comment on a line",
            description: "Adds a line comment to the attempt's changes (the Changes view), on the file as it is in \
                          the attempt's worktree.",
            schema: schema::<OutcomeComment>,
        },
        Def {
            name: "update_ticket",
            title: "Update the ticket",
            description: "Rewrites this session's ticket: its title and/or description (as `PATCH /api/tickets/{id}`, \
                          recorded as the agent's edit). Only this session's own ticket.",
            schema: schema::<UpdateTicket>,
        },
        Def {
            name: "catalog_search",
            title: "Search the node catalog",
            description: "Lists workflow node kinds (triggers, conditions, actions, gates, waits) matching the \
                          words: id, title, category, description. catalog_get gives one kind's schemas.",
            schema: schema::<CatalogSearch>,
        },
        Def {
            name: "catalog_get",
            title: "Get a node kind",
            description: "One node kind with its config and output JSON Schemas and its ports.",
            schema: schema::<CatalogGet>,
        },
        Def {
            name: "workflow_get",
            title: "Get a workflow",
            description: "A workflow (by id or name) with its current document and validation issues.",
            schema: schema::<WorkflowGet>,
        },
        Def {
            name: "workflow_validate",
            title: "Validate a workflow",
            description: "Checks a workflow document: errors and warnings.",
            schema: schema::<crate::workflow::routes::ValidateRequest>,
        },
        Def {
            name: "workflow_dry_run",
            title: "Dry-run a workflow",
            description: "Runs a document (or a saved workflow) against an event in memory, changing nothing: which \
                          triggers match and what each step would do.",
            schema: schema::<crate::workflow::routes::DryRunRequest>,
        },
        Def {
            name: "workflow_save",
            title: "Save a workflow",
            description: "Proposes a document as a new version of a workflow (the user reviews and accepts it), or \
                          saves it as a new workflow, which starts disabled: the user reviews and enables it.",
            schema: schema::<WorkflowSave>,
        },
    ]
}

pub(crate) fn infos() -> Vec<ToolInfo> {
    defs()
        .into_iter()
        .map(|d| ToolInfo {
            name: d.name.into(),
            title: d.title.into(),
            description: d.description.into(),
            input_schema: (d.schema)(),
            annotations: json!({"title": d.title, "readOnlyHint": READ_ONLY.contains(&d.name),
                                "destructiveHint": false, "openWorldHint": false}),
        })
        .collect()
}

fn args<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    let v = if v.is_null() { json!({}) } else { v };
    serde_json::from_value(v).map_err(|e| format!("the arguments don't fit the tool's schema: {e}"))
}

fn api(e: ApiError) -> String {
    e.message().to_string()
}

fn store(e: pdt_store::StoreError) -> String {
    ApiError::from(e).message().to_string()
}

/// Runs a tool. Err is a message for the agent.
pub(crate) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "get_ticket" => {
            let a: GetTicket = args(a)?;
            let id = ticket_for(cx, a.ticket.as_deref()).await?;
            get_ticket(cx, &id).await
        }
        "submit_outcome" => {
            let a: SubmitOutcome = args(a)?;
            crate::workflow::agent::submit_outcome(st, cx.session, a).await
        }
        "ask_user" => {
            let a: AskUser = args(a)?;
            crate::workflow::agent::ask_user(st, cx.session, a.questions).await
        }
        "add_ticket_comment" => {
            let a: AddTicketComment = args(a)?;
            let id = ticket_for(cx, a.ticket.as_deref()).await?;
            let body = valid_comment(&a.body).map_err(|e| api(e.into()))?;
            let data = json!({"sessionId": cx.session.id, "persona": cx.session.persona.as_ref().map(|p| &p.name)});
            let e = st
                .store
                .add_comment_by(&id, &body, Actor::Agent, Some(data), now_ms())
                .await
                .map_err(store)?
                .ok_or("that ticket no longer exists")?;
            Ok(json!({"commented": true, "eventId": e.id}))
        }
        "update_ticket" => {
            let a: UpdateTicket = args(a)?;
            if a.title.is_none() && a.description.is_none() {
                return Err("give a title, a description or both".into());
            }
            let id = ticket_for(cx, None).await?;
            let patch =
                pdt_core::ticket::TicketPatch { title: a.title, description: a.description, ..Default::default() };
            let valid = patch.validate().map_err(|e| api(e.into()))?;
            let t = st
                .store
                .patch_ticket_by(&id, &valid, Actor::Agent, now_ms())
                .await
                .map_err(|e| api(e.into()))?
                .ok_or("that ticket no longer exists")?;
            Ok(json!({"updated": true, "key": t.summary.key, "title": t.summary.title}))
        }
        "add_review_comment" => {
            let a: OutcomeComment = args(a)?;
            let c = add_review_comment(cx, &a).await?;
            Ok(json!({"commented": true, "commentId": c.id, "file": c.path, "line": c.line}))
        }
        "catalog_search" => {
            let a: CatalogSearch = args(a)?;
            let words: Vec<String> =
                a.query.unwrap_or_default().to_lowercase().split_whitespace().map(String::from).collect();
            let kinds: Vec<Value> = catalog()
                .specs()
                .into_iter()
                .filter(|k| a.category.is_none_or(|c| c == k.category))
                .filter(|k| {
                    let text = format!("{} {} {}", k.id, k.title, k.description).to_lowercase();
                    words.iter().all(|w| text.contains(w.as_str()))
                })
                .map(|k| json!({"id": k.id, "title": k.title, "category": k.category, "description": k.description}))
                .collect();
            Ok(json!({"kinds": kinds}))
        }
        "catalog_get" => {
            let a: CatalogGet = args(a)?;
            let k = catalog().spec(&a.id).ok_or_else(|| format!("no node kind is called {}", a.id))?;
            serde_json::to_value(k).map_err(|e| e.to_string())
        }
        "workflow_get" => {
            let a: WorkflowGet = args(a)?;
            let id = match (a.id, a.name) {
                (Some(id), _) => id,
                (None, Some(name)) => st
                    .store
                    .list_workflows(Some(&cx.session.project_id))
                    .await
                    .map_err(store)?
                    .into_iter()
                    .find(|w| w.name.eq_ignore_ascii_case(name.trim()))
                    .map(|w| w.id)
                    .ok_or_else(|| format!("no workflow is called {name}"))?,
                (None, None) => return Err("give the workflow's id or name".into()),
            };
            let w = crate::workflow::routes::load(st, &id).await.map_err(api)?;
            let d = crate::workflow::routes::detail(st, w).await.map_err(api)?;
            serde_json::to_value(d).map_err(|e| e.to_string())
        }
        "workflow_validate" => {
            let a: crate::workflow::routes::ValidateRequest = args(a)?;
            let Json(issues) = crate::workflow::routes::validate_doc(Ok(Json(a))).await.map_err(api)?;
            Ok(json!({"issues": issues}))
        }
        "workflow_dry_run" => {
            let a: crate::workflow::routes::DryRunRequest = args(a)?;
            let Json(r) = crate::workflow::routes::dry_run(State(st.clone()), Ok(Json(a))).await.map_err(api)?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }
        "workflow_save" => {
            let a: WorkflowSave = args(a)?;
            let d = match a.id {
                // An agent's change to a workflow is a proposal the user reviews (Phase 7d spec §6.2).
                Some(id) => {
                    let s = crate::workflow::routes::SaveWorkflow { doc: Some(a.doc), note: a.note, propose: true };
                    let Json(d) =
                        crate::workflow::routes::save(State(st.clone()), axum::extract::Path(id), Ok(Json(s)))
                            .await
                            .map_err(api)?;
                    d
                }
                None => {
                    let scope = a.scope.unwrap_or(Scope::Project);
                    let project_id = match scope {
                        Scope::Project => Some(a.project_id.unwrap_or_else(|| cx.session.project_id.clone())),
                        _ => None,
                    };
                    crate::workflow::routes::create_workflow(
                        st,
                        &a.doc,
                        Some(scope),
                        project_id,
                        false,
                        &a.note,
                        Author::Agent,
                    )
                    .await
                    .map_err(api)?
                }
            };
            Ok(json!({"workflowId": d.workflow.id, "version": d.workflow.version, "proposed": d.workflow.proposed,
                      "enabled": d.workflow.enabled, "issues": d.issues}))
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// The ticket a tool means: `given` (a key or id), else the session's (its attempt's, or its agent step's).
async fn ticket_for(cx: &Cx<'_>, given: Option<&str>) -> Result<String, String> {
    let st = cx.st;
    if let Some(g) = given.map(str::trim).filter(|g| !g.is_empty()) {
        if st.store.get_ticket(g).await.map_err(store)?.is_some() {
            return Ok(g.to_string());
        }
        let k = pdt_core::ticket::parse_key(g).ok_or_else(|| format!("{g} is not a ticket key or id"))?;
        let project = match &k.key {
            Some(key) => st
                .store
                .list_projects()
                .await
                .map_err(store)?
                .into_iter()
                .find(|p| p.key.eq_ignore_ascii_case(key))
                .map(|p| p.id)
                .ok_or_else(|| format!("no project has the key {key}"))?,
            None => cx.session.project_id.clone(),
        };
        return st
            .store
            .ticket_id_by_number(&project, k.number)
            .await
            .map_err(store)?
            .ok_or_else(|| format!("there is no ticket {g}"));
    }
    session_ticket(st, cx.session)
        .await
        .ok_or_else(|| "this session works on no ticket; name one (`ticket`: a key such as PDT-12)".to_string())
}

/// The ticket a session works on: its attempt's, else its agent step's.
pub(crate) async fn session_ticket(st: &AppState, s: &Session) -> Option<String> {
    if let Some(t) = &s.ticket {
        return Some(t.id.clone());
    }
    st.store.active_agent_step(&s.id).await.ok().flatten().and_then(|a| a.ticket_id)
}

async fn get_ticket(cx: &Cx<'_>, id: &str) -> Result<Value, String> {
    let st = cx.st;
    let Json(ticket) =
        crate::tickets::get_one(State(st.clone()), axum::extract::Path(id.to_string())).await.map_err(api)?;
    let comments: Vec<Value> = st
        .store
        .comments(id)
        .await
        .map_err(store)?
        .into_iter()
        .map(|(actor, body, at)| json!({"by": actor, "body": body, "at": at}))
        .collect();
    let outcomes: Vec<Value> = st
        .store
        .ticket_outcomes(id)
        .await
        .map_err(store)?
        .into_iter()
        .map(|a| {
            json!({"persona": a.persona_name, "outcome": a.outcome, "summary": a.summary, "comments": a.comments,
                   "by": if a.by_user { "user" } else { "agent" }, "at": a.updated_at})
        })
        .collect();
    let step = match st.store.active_agent_step(&cx.session.id).await.map_err(store)? {
        Some(a) if a.ticket_id.as_deref() == Some(id) => {
            json!({"persona": a.persona_name, "outcomes": a.outcomes, "state": a.state})
        }
        _ => Value::Null,
    };
    let mut v = serde_json::to_value(ticket).map_err(|e| e.to_string())?;
    v["comments"] = json!(comments);
    v["outcomes"] = json!(outcomes);
    if !step.is_null() {
        v["step"] = step;
    }
    Ok(v)
}

/// A review comment on the session's attempt's worktree, by its persona (or `agent`), already sent.
pub(crate) async fn add_review_comment(
    cx: &Cx<'_>,
    c: &OutcomeComment,
) -> Result<pdt_core::review::ReviewComment, String> {
    let st = cx.st;
    let attempt = match &cx.session.attempt_id {
        Some(a) => Some(a.clone()),
        None => st.store.active_agent_step(&cx.session.id).await.map_err(store)?.and_then(|a| a.attempt_id),
    };
    let attempt = attempt.ok_or("this session works on no attempt, so there are no changes to comment on")?;
    let info = st.store.get_attempt(&attempt).await.map_err(store)?.ok_or("the attempt is gone")?;
    let wt_id = info.attempt.worktree_id.clone().ok_or("the attempt has no worktree")?;
    let wt = st.store.get_worktree(&wt_id).await.map_err(store)?.ok_or("the attempt's worktree is gone")?;
    let end = c.end_line.unwrap_or(c.line).max(c.line);
    let file = crate::files::clean_path(&c.file).map_err(api)?;
    let text = std::fs::read_to_string(std::path::Path::new(&wt.path).join(&file)).unwrap_or_default();
    let line_text: String = text
        .lines()
        .skip(usize::try_from(c.line.saturating_sub(1)).unwrap_or(0))
        .take(usize::try_from(end - c.line + 1).unwrap_or(1))
        .collect::<Vec<_>>()
        .join("\n");
    let input = NewReviewComment {
        path: file,
        side: CommentSide::New,
        line: c.line,
        end_line: Some(end),
        line_text,
        body: c.body.clone(),
        parent_id: None,
    };
    let author = cx.session.persona.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "agent".into());
    crate::review::add_comment(st, &wt_id, input, &author, true).await.map_err(api)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_listed_once_with_an_object_schema() {
        let names: Vec<String> = infos().into_iter().map(|t| t.name).collect();
        assert_eq!(names, pdt_core::persona::ALL_TOOLS.to_vec());
        for t in infos() {
            assert_eq!(t.input_schema["type"], "object", "{}", t.name);
            assert!(t.input_schema.get("$schema").is_none());
        }
        let s = &infos()[1].input_schema;
        assert!(s["required"].as_array().unwrap().iter().any(|r| r == "outcome"), "{s}");
    }
}
