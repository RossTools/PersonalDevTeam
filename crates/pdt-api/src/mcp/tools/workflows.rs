//! The workflow tools: the node catalog, and reading, checking, saving and running workflows.

use super::*;

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
struct WorkflowRun {
    #[serde(default)]
    id: Option<String>,
    /// The workflow's name, when you don't know its id.
    #[serde(default)]
    name: Option<String>,
    /// The project it runs in (its key or id): its `projectId` in the run's input.
    project: String,
    /// The run's input, read as `{{trigger.x}}` (the workflow's description says what it reads).
    #[serde(default)]
    input: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowExport {
    /// The workflows' ids.
    #[serde(default)]
    ids: Vec<String>,
    /// Or their names, when you don't know the ids.
    #[serde(default)]
    names: Vec<String>,
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

pub(super) fn defs() -> Vec<Tool> {
    vec![
        Tool {
            decl: ToolDecl::new("catalog_search", "Search the workflow node catalog").read_only().pm(),
            title: "Search the node catalog",
            description: "Lists workflow node kinds (triggers, conditions, actions, gates, waits) matching the \
                          words: id, title, category, description. MCP servers' tools are listed too, as \
                          `mcp:<server>/<tool>` (a `preset` mcp.tool node). catalog_get gives one kind's schemas.",
            schema: schema::<CatalogSearch>,
        },
        Tool {
            decl: ToolDecl::new("catalog_get", "Read a catalog node").read_only().pm(),
            title: "Get a node kind",
            description: "One node kind with its config and output JSON Schemas and its ports.",
            schema: schema::<CatalogGet>,
        },
        Tool {
            decl: ToolDecl::new("workflow_get", "Read a workflow").read_only().pm(),
            title: "Get a workflow",
            description: "A workflow (by id or name) with its current document and validation issues, and runInputs: \
                          the fields its Run now trigger declares (what workflow_run takes).",
            schema: schema::<WorkflowGet>,
        },
        Tool {
            decl: ToolDecl::new("workflow_validate", "Check a workflow document").read_only().pm(),
            title: "Validate a workflow",
            description: "Checks a workflow document: errors and warnings.",
            schema: schema::<crate::workflow::routes::ValidateRequest>,
        },
        Tool {
            decl: ToolDecl::new("workflow_dry_run", "Try a workflow without running it").read_only().pm(),
            title: "Dry-run a workflow",
            description: "Runs a document (or a saved workflow) against an event in memory, changing nothing: which \
                          triggers match and what each step would do.",
            schema: schema::<crate::workflow::routes::DryRunRequest>,
        },
        Tool {
            decl: ToolDecl::new("workflow_save", "Save a workflow (a new version)").pm(),
            title: "Save a workflow",
            description: "Proposes a document as a new version of a workflow (the user reviews and accepts it), or \
                          saves it as a new workflow, which starts disabled: the user reviews and enables it.",
            schema: schema::<WorkflowSave>,
        },
        Tool {
            decl: ToolDecl::new("workflow_export", "Export workflows as text to paste into another PDT")
                .read_only()
                .pm(),
            title: "Export workflows",
            description: "Workflows' current documents as one export text (format pdt-workflow): project ids become \
                          {\"$project\": \"<KEY>\"} placeholders and `requires` lists the connections, personas and \
                          projects they need. Paste it into Import on another PDT, or give it to workflow_import there.",
            schema: schema::<WorkflowExport>,
        },
        Tool {
            decl: ToolDecl::new("workflow_run", "Run an enabled workflow now").asks().pm(),
            title: "Run a workflow",
            description: "Runs an enabled workflow that has a Run now trigger, in a project, with an input its steps \
                          read as {{trigger.x}}: e.g. {\"environment\": \"staging\"} for a deploy workflow. When the \
                          trigger declares inputs (workflow_get's runInputs), the input must be those fields: defaults \
                          fill in, and anything else is refused with the list; without, its description says what it \
                          reads. The user confirms each call. Returns the run's id (list_workflow_runs follows it).",
            schema: schema::<WorkflowRun>,
        },
        Tool {
            decl: ToolDecl::new("workflow_import", "Import pasted workflows (disabled)").asks().pm(),
            title: "Import workflows",
            description: "Imports an export text (or a bare workflow document). With `check: true` it only reports \
                          the workflows, names that clash, and how each connection, persona and project they need \
                          stands here (found, missing, kind_mismatch). Then `mapping` maps each missing one to a local \
                          name (a project key to a local project's key or id) or null to leave it unmapped, and \
                          `targets` (one per workflow, in order) give scope/projectId and, on a name clash, `copy` or \
                          `versionOf` (a proposed version). New workflows start disabled. The user confirms each call.",
            schema: schema::<pdt_core::workflow::transfer::ImportRequest>,
        },
    ]
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "catalog_search" => {
            let a: CatalogSearch = args(a)?;
            let words: Vec<String> =
                a.query.unwrap_or_default().to_lowercase().split_whitespace().map(String::from).collect();
            let kinds: Vec<Value> = crate::workflow::catalog::listed(st)
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
            let k = crate::workflow::catalog::listed_spec(st, &a.id)
                .ok_or_else(|| format!("no node kind is called {}", a.id))?;
            serde_json::to_value(k).map_err(|e| e.to_string())
        }
        "workflow_get" => {
            let a: WorkflowGet = args(a)?;
            let id = match (a.id, a.name) {
                (Some(id), _) => id,
                (None, Some(name)) => workflow_by_name(cx, &name).await?,
                (None, None) => return Err("give the workflow's id or name".into()),
            };
            let w = crate::workflow::routes::load(st, &id).await.map_err(api)?;
            let d = crate::workflow::routes::detail(st, w).await.map_err(api)?;
            let mut v = serde_json::to_value(&d).map_err(|e| e.to_string())?;
            // What workflow_run takes: the Run now trigger's declared inputs (PDT-301).
            if let Some(n) = d.doc.nodes.iter().find(|n| n.kind == crate::workflow::nodes::trigger::MANUAL) {
                let c = st.registry().catalog().current_config(n).unwrap_or_default();
                v["runInputs"] = c.get("inputs").cloned().unwrap_or_else(|| json!([]));
            }
            Ok(v)
        }
        "workflow_validate" => {
            let a: crate::workflow::routes::ValidateRequest = args(a)?;
            let Json(issues) =
                crate::workflow::routes::validate_doc(State(st.clone()), Ok(Json(a))).await.map_err(api)?;
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
                    let by =
                        cx.session.persona.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| cx.session.title.clone());
                    let by = Some(by).filter(|b| !b.trim().is_empty());
                    let s = crate::workflow::routes::SaveWorkflow { doc: Some(a.doc), note: a.note, propose: true, by };
                    let Json(d) =
                        crate::workflow::routes::save(State(st.clone()), axum::extract::Path(id), Ok(Json(s)))
                            .await
                            .map_err(api)?;
                    d
                }
                None => {
                    let scope = a.scope.unwrap_or(Scope::Project);
                    let project_id = match (scope, a.project_id) {
                        (Scope::Project, Some(p)) => Some(crate::pm::project_id(st, &p).await?),
                        // The PM is in no project: it names one.
                        (Scope::Project, None) if crate::pm::is_pm_session(st, cx.session).await => {
                            return Err("name the workflow's project (`projectId`: its key or id)".into());
                        }
                        (Scope::Project, None) => Some(cx.session.project_id.clone()),
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
        "workflow_export" => {
            let a: WorkflowExport = args(a)?;
            let mut ids = a.ids;
            for n in &a.names {
                ids.push(workflow_by_name(cx, n).await?);
            }
            if ids.is_empty() {
                return Err("give the workflows' ids or names".into());
            }
            let e = crate::workflow::transfer::export(st, &ids).await.map_err(api)?;
            Ok(json!({"text": pdt_core::workflow::transfer::export_text(&e), "workflows": e.workflows.len(),
                      "requires": e.requires}))
        }
        "workflow_run" => {
            let a: WorkflowRun = args(a)?;
            let id = match (a.id, a.name) {
                (Some(id), _) => id,
                (None, Some(name)) => workflow_by_name(cx, &name).await?,
                (None, None) => return Err("give the workflow's id or name".into()),
            };
            let w = crate::workflow::routes::load(st, &id).await.map_err(api)?;
            if !w.enabled {
                return Err(format!("{} is off: the user turns it on in Workflows first", w.name));
            }
            let project = crate::pm::project_id(st, &a.project).await?;
            confirmable(cx, name, &AskTarget::Project(project.clone())).await?;
            let mut input = a.input;
            input.insert("projectId".into(), json!(project));
            let run = crate::workflow::engine::start_manual(st, &w, Value::Object(input), None).await?;
            Ok(json!({"runId": run, "workflow": w.name}))
        }
        "workflow_import" => {
            let a: pdt_core::workflow::transfer::ImportRequest = args(a)?;
            if a.check {
                let c = crate::workflow::transfer::check(st, &a.text).await.map_err(api)?;
                return serde_json::to_value(c).map_err(|e| e.to_string());
            }
            confirmable(cx, name, &AskTarget::Always).await?;
            let r = crate::workflow::transfer::import(st, &a, Author::Agent).await.map_err(api)?;
            serde_json::to_value(r).map_err(|e| e.to_string())
        }
        other => Err(format!("no tool is called {other}")),
    }
}

/// A workflow's id from its name: in the session's project (with host-wide and personal ones), or for the PM anywhere.
async fn workflow_by_name(cx: &Cx<'_>, name: &str) -> Result<String, String> {
    let st = cx.st;
    st.store
        // The PM looks across every project.
        .list_workflows(match crate::pm::is_pm_session(st, cx.session).await {
            true => None,
            false => Some(&cx.session.project_id),
        })
        .await
        .map_err(store)?
        .into_iter()
        .filter(|w| !crate::workflow::builtins::hidden(&st.registry(), w))
        .find(|w| w.name.eq_ignore_ascii_case(name.trim()))
        .map(|w| w.id)
        .ok_or_else(|| format!("no workflow is called {name}"))
}
