//! Ticket pickup (PDT-327): `tickets.candidates` (a project's tickets that could start, in order), `tickets.choose`
//! (caps, the start delay, failed starts, overlap with work in flight; the rules are pdt-core `pickup`) and
//! `ticket.start` (Start, as auto-start).
//! The built-in "Pick up tickets" chains them; a field left out takes the project's auto-start setting.

use crate::routes::AppState;
use crate::start::Starter;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use pdt_core::pickup::{
    ActiveTicket, CandidateFilter, ChooseRules, DEFAULT_LIMIT, OverlapOverride, OverlapPolicy, OverlapRule,
    PickupOrder, PickupProject, PickupTicket, choose,
};
use pdt_core::policy::AutoStartOrder;
use pdt_core::start::{StartMode, StartTicket};
use pdt_core::ticket::{AttemptState, Priority, TicketQuery, TicketStage};
use pdt_core::workflow::api::Scope;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use ts_rs::TS;

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct CandidatesConfig {
    /// The project's id (`{{trigger.projectId}}`); empty: every project the workflow covers (its own, or every project
    /// for a This host workflow).
    #[ts(optional)]
    pub project: Option<String>,
    /// Only these stages, in this order (`["todo", "ready"]`: To do first). Left out: the project's auto-start
    /// setting (none while it is off).
    #[ts(optional)]
    pub stages: Option<Vec<TicketStage>>,
    /// Only tickets with any of these labels.
    pub labels: Vec<String>,
    /// No ticket with any of these labels.
    pub exclude_labels: Vec<String>,
    /// Only tickets at least this urgent.
    #[ts(optional)]
    pub min_priority: Option<Priority>,
    /// Only tickets in any of these areas.
    pub areas: Vec<String>,
    /// Leave out tickets with an open blocker.
    #[serde(default = "yes")]
    pub exclude_blocked: bool,
    /// Leave out parents (otherwise starting a parent starts its next part).
    #[serde(default = "yes")]
    pub exclude_parents: bool,
    /// Leave out tickets that conflict with work in flight.
    #[serde(default = "yes")]
    pub exclude_conflicts: bool,
    /// `priority`, `rank` or `age` (oldest first). Left out: the project's auto-start order.
    #[ts(optional)]
    pub order: Option<PickupOrder>,
    /// The most candidates per project (default 50).
    #[ts(optional)]
    pub limit: Option<u32>,
}

impl Default for CandidatesConfig {
    fn default() -> Self {
        CandidatesConfig {
            project: None,
            stages: None,
            labels: vec![],
            exclude_labels: vec![],
            min_priority: None,
            areas: vec![],
            exclude_blocked: true,
            exclude_parents: true,
            exclude_conflicts: true,
            order: None,
            limit: None,
        }
    }
}

impl CandidatesConfig {
    /// The filter for a project with `policy`: the step's fields, else the project's auto-start settings.
    fn filter(&self, policy: &pdt_core::policy::ProjectPolicy) -> CandidateFilter {
        let base = CandidateFilter::from_policy(policy);
        CandidateFilter {
            stages: self.stages.clone().unwrap_or(base.stages),
            labels: self.labels.clone(),
            exclude_labels: self.exclude_labels.clone(),
            min_priority: self.min_priority,
            areas: self.areas.clone(),
            exclude_blocked: self.exclude_blocked,
            exclude_parents: self.exclude_parents,
            exclude_conflicts: self.exclude_conflicts,
            order: self.order.unwrap_or(base.order),
            limit: self.limit.map_or(DEFAULT_LIMIT, |l| usize::try_from(l).unwrap_or(DEFAULT_LIMIT)),
        }
    }
}

/// What `tickets.candidates` outputs.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CandidatesOutput {
    pub projects: Vec<PickupProject>,
    /// How many candidates in all.
    pub count: usize,
}

pub struct Candidates;

impl NodeKind for Candidates {
    fn spec(&self) -> KindSpec {
        spec::<CandidatesConfig, CandidatesOutput>(
            "tickets.candidates",
            1,
            Category::Action,
            "Find tickets to start",
            "Lists the tickets of a project (or of every project the workflow covers) that could start, in order: no \
             active attempt, not waiting, in the given stages, filtered by labels, priority and areas. Each project \
             comes with its auto-start cap, start delay, overlap rule (its hot areas) and tickets in flight (their \
             labels and areas). Fields left \
             out take the project's auto-start settings; a project with no stages is left out.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<CandidatesConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CandidatesConfig = parse(&input)?;
            let projects = covered_projects(cx, c.project.as_deref()).await?;
            let mut out = vec![];
            for p in projects {
                let policy = cx.st.store.get_project_settings(&p.id).await.map_err(|e| e.to_string())?.policy;
                let filter = c.filter(&policy);
                if filter.stages.is_empty() {
                    continue;
                }
                let sort = match filter.order {
                    PickupOrder::Priority => AutoStartOrder::Priority,
                    PickupOrder::Rank | PickupOrder::Age => AutoStartOrder::Rank,
                };
                let stages: Vec<&str> = filter.stages.iter().map(|s| s.as_str()).collect();
                let q = TicketQuery {
                    project: Some(p.id.clone()),
                    stage: Some(stages.join(",")),
                    has_parts: filter.exclude_parents.then_some(false),
                    sort: Some(sort.ticket_sort()),
                    limit: Some(200),
                    ..Default::default()
                };
                let q = q.validate().map_err(|e| e.to_string())?;
                let idx = crate::overlap::index(cx.st, Some(&p.id)).await.map_err(|e| e.message().to_string())?;
                let mut page = cx.st.store.query_tickets(&q).await.map_err(|e| e.to_string())?;
                for t in page.items.iter_mut() {
                    idx.apply_summary(t);
                }
                let tickets = filter.apply(&page.items).into_iter().map(PickupTicket::from).collect();
                out.push(PickupProject {
                    project_id: p.id.clone(),
                    key: p.key.clone(),
                    name: p.name.clone(),
                    cap: policy.auto_start_cap(),
                    start_delay_seconds: policy.start_delay_seconds,
                    overlap: OverlapPolicy::from_policy(&policy),
                    active: in_flight(cx.st, &p.id).await?,
                    tickets,
                });
            }
            let count = out.iter().map(|p| p.tickets.len()).sum::<usize>();
            let mut v = json!({"projects": out, "count": count});
            // Finding nothing to do is nothing to report.
            v["quiet"] = json!(true);
            Ok(Outcome::Next { port: "out".into(), output: v })
        })
    }
}

/// The projects a step works on: the one named, else the workflow's own, else (a This host workflow) every project.
async fn covered_projects(cx: &StepCx<'_>, named: Option<&str>) -> Result<Vec<pdt_core::project::Project>, String> {
    let get = |id: String| async move {
        cx.st.store.get_project(&id).await.map_err(|e| e.to_string())?.ok_or_else(|| format!("no project {id}"))
    };
    if let Some(id) = named.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(vec![get(id.to_string()).await?]);
    }
    match (cx.workflow.scope, &cx.workflow.project_id) {
        (Scope::Project, Some(id)) => Ok(vec![get(id.clone()).await?]),
        (Scope::Host, _) => cx.st.store.list_projects().await.map_err(|e| e.to_string()),
        _ => Ok(vec![]),
    }
}

/// The project's tickets with an active attempt, with their labels and areas.
async fn in_flight(st: &AppState, project_id: &str) -> Result<Vec<ActiveTicket>, String> {
    let mut out = vec![];
    for a in st.store.list_active_attempts().await.map_err(|e| e.to_string())? {
        if a.project_id != project_id {
            continue;
        }
        let t = st.store.get_ticket_summary(&a.ticket_id).await.map_err(|e| e.to_string())?;
        let (labels, areas) = t.map(|t| (t.labels, t.areas)).unwrap_or_default();
        out.push(ActiveTicket { id: a.ticket_id, key: a.ticket_key, labels, areas });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct ChooseConfig {
    /// The candidates step's output (`{{steps.candidates}}`).
    #[ts(type = "unknown")]
    pub candidates: Value,
    /// The most tickets in flight per project. Left out: the project's auto-start cap (its running cap, else 1).
    #[ts(optional)]
    pub cap: Option<u32>,
    /// The most tickets in flight in an area (`{"web": 1}`).
    pub area_caps: BTreeMap<String, u32>,
    /// The most tickets in flight with a label (`{"agentic": 2}`).
    pub label_caps: BTreeMap<String, u32>,
    /// How long a ticket goes unchanged first. Left out: the project's start delay.
    #[ts(optional)]
    pub start_delay_seconds: Option<u32>,
    /// Pass over a ticket that failed to start until it changes.
    #[serde(default = "yes")]
    pub skip_failed: bool,
    /// How candidates overlapping work in flight are treated: `off` (only candidates' conflicts), `avoid` (pass over
    /// a candidate in a hot area while a ticket in it is in flight) or `serialize` (at most `perArea` in flight in
    /// each area). Left out: `avoid` when the project has hot areas, else `off`.
    #[ts(optional)]
    pub overlap: Option<OverlapRule>,
    /// The hot areas. Left out: the project's.
    #[ts(optional)]
    pub hot_areas: Option<Vec<String>>,
    /// `serialize`: the most tickets in flight in one area (default 1).
    #[ts(optional)]
    pub per_area: Option<u32>,
    /// A ticket without areas may start alongside a hot area's ticket in flight, and the other way round. Left out:
    /// the project's setting.
    #[ts(optional)]
    pub untagged_alongside_hot: Option<bool>,
}

impl Default for ChooseConfig {
    fn default() -> Self {
        ChooseConfig {
            candidates: Value::Null,
            cap: None,
            area_caps: BTreeMap::new(),
            label_caps: BTreeMap::new(),
            start_delay_seconds: None,
            skip_failed: true,
            overlap: None,
            hot_areas: None,
            per_area: None,
            untagged_alongside_hot: None,
        }
    }
}

pub struct Choose;

impl NodeKind for Choose {
    fn spec(&self) -> KindSpec {
        spec::<ChooseConfig, Value>(
            "tickets.choose",
            1,
            Category::Condition,
            "Choose tickets to start",
            "From the candidates, the tickets that may start now: under the project's cap (and any area or label \
             caps), unchanged for the start delay, not a ticket that failed to start and hasn't changed since, and \
             clear of work in flight under the overlap rule (by default, nothing in a hot area while another ticket \
             in it is in flight; the Board shows why a ticket waits). \
             `chosen`: `ticket` (the first) and `tickets` (every one, each project's in order: Start takes the first \
             still free). `none`: nothing may start. Both list the tickets passed over with why (`passed`).",
            &[("chosen", "Chosen"), ("none", "None")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<ChooseConfig>(config).map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: ChooseConfig = parse(&input)?;
            let projects: Vec<PickupProject> = match c.candidates.get("projects") {
                Some(p) => serde_json::from_value(p.clone()).map_err(|e| format!("candidates: {e}"))?,
                None => return Err("candidates is not a candidates step's output ({{steps.<id>}})".into()),
            };
            let rules = ChooseRules {
                cap: c.cap,
                area_caps: c.area_caps,
                label_caps: c.label_caps,
                start_delay_seconds: c.start_delay_seconds,
                skip_failed: c.skip_failed,
                overlap: OverlapOverride {
                    rule: c.overlap,
                    hot_areas: c.hot_areas,
                    per_area: c.per_area,
                    untagged_alongside_hot: c.untagged_alongside_hot,
                },
            };
            let failed = cx.st.auto_start_failed.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let choice = choose(&projects, &rules, &failed, crate::routes::now_ms());
            if !cx.dry_run {
                record_holds(cx.st, &projects, &choice.passed, crate::routes::now_ms());
            }
            match choice.chosen.first() {
                Some(first) => Ok(Outcome::next(
                    "chosen",
                    json!({"ticket": first, "tickets": choice.chosen, "passed": choice.passed}),
                )),
                None => Ok(Outcome::next("none", json!({"passed": choice.passed}))),
            }
        })
    }
}

/// Keeps why choose passed over each of `projects`' candidates for overlap, for the Board, in place of what the
/// last choose said of those projects.
fn record_holds(st: &AppState, projects: &[PickupProject], passed: &[pdt_core::pickup::Passed], now: i64) {
    let mut holds = st.pickup_holds.lock().unwrap_or_else(|e| e.into_inner());
    holds.retain(|_, h| !projects.iter().any(|p| p.project_id == h.project_id));
    for p in projects {
        for t in &p.tickets {
            if let Some(x) = passed.iter().find(|x| x.id == t.id && x.reason.is_overlap()) {
                let hold =
                    crate::scheduler::PickupHold { project_id: p.project_id.clone(), why: x.why.clone(), at: now };
                holds.insert(t.id.clone(), hold);
            }
        }
    }
}

/// Who takes a started ticket over.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HandTo {
    /// The workflow that runs on `attempt.started` (the Pipeline) when the project has one; else its own agent session.
    #[default]
    Auto,
    /// Its own agent session, whatever workflows there are.
    Session,
    /// No session: a workflow on `attempt.started` takes it over.
    Workflow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct StartConfig {
    /// What to start: a ticket's id, or choose's tickets (`{{steps.choose.tickets}}`), of which the first of each
    /// project still free starts.
    #[ts(type = "unknown")]
    pub ticket: Value,
    /// `claude`, `codex`, `devin` or `acp:<id>`. Left out: the host's default agent.
    #[ts(optional)]
    pub agent: Option<String>,
    /// The session's permission mode. Left out: the host's for tickets.
    #[ts(optional)]
    pub permission_mode: Option<String>,
    /// `conversation` or `terminal`. Left out: the host's default.
    #[ts(optional)]
    pub session_mode: Option<String>,
    /// Who takes it over: `auto`, `session` or `workflow`.
    pub hand_to: HandTo,
    /// How many tickets of one project it starts at most (default 1).
    #[ts(optional)]
    pub per_project: Option<u32>,
}

impl Default for StartConfig {
    fn default() -> Self {
        StartConfig {
            ticket: Value::Null,
            agent: None,
            permission_mode: None,
            session_mode: None,
            hand_to: HandTo::Auto,
            per_project: None,
        }
    }
}

impl StartConfig {
    fn fields(&self) -> Result<StartFields, String> {
        fn named<T: serde::de::DeserializeOwned>(what: &str, v: &Option<String>) -> Result<Option<T>, String> {
            match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(s) => serde_json::from_value(Value::String(s.into()))
                    .map(Some)
                    .map_err(|_| format!("{what} {s:?} is not one")),
                None => Ok(None),
            }
        }
        Ok(StartFields {
            agent: named("agent", &self.agent)?,
            permission_mode: named("permission mode", &self.permission_mode)?,
            session_mode: named("session mode", &self.session_mode)?,
        })
    }

    /// The tickets to try, in order, each with the caps it was chosen under (choose's picks; none for an id).
    fn tickets(&self) -> Result<Vec<(String, Option<Caps>)>, String> {
        fn one(v: &Value) -> Result<(String, Option<Caps>), String> {
            match v {
                Value::String(s) if !s.trim().is_empty() => Ok((s.trim().to_string(), None)),
                Value::Object(m) => {
                    let id = m.get("id").and_then(Value::as_str).ok_or("a ticket without an id")?;
                    let caps = match m.get("cap") {
                        Some(_) => Some(serde_json::from_value::<Caps>(v.clone()).map_err(|e| format!("a pick: {e}"))?),
                        None => None,
                    };
                    Ok((id.to_string(), caps))
                }
                _ => Err("ticket is not a ticket's id, nor choose's tickets".into()),
            }
        }
        match &self.ticket {
            Value::Array(list) => list.iter().map(one).collect(),
            v => one(v).map(|t| vec![t]),
        }
    }
}

/// The caps a pick was chosen under (`pickup::Pick`'s), checked again under the pickup lock.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Caps {
    cap: u32,
    #[serde(default)]
    area_caps: BTreeMap<String, u32>,
    #[serde(default)]
    label_caps: BTreeMap<String, u32>,
    #[serde(default)]
    overlap_rule: OverlapPolicy,
}

struct StartFields {
    agent: Option<pdt_core::session::AgentKind>,
    permission_mode: Option<pdt_core::conversation::PermissionMode>,
    session_mode: Option<pdt_core::conversation::SessionMode>,
}

pub struct Start;

/// `ticket.start`'s progress: the ticket it is starting, so a run again after a restart counts it started.
const TRYING: &str = "trying";

impl NodeKind for Start {
    fn spec(&self) -> KindSpec {
        spec::<StartConfig, Value>(
            "ticket.start",
            1,
            Category::Action,
            "Start the ticket",
            "Starts a ticket as auto-start does (a branch, a worktree and an attempt), with the host's default agent, \
             permission mode and view unless given; handed to the workflow on `attempt.started` (the Pipeline) or an \
             agent session of its own. Given choose's tickets, it starts the first of each project still free (no \
             active attempt, under the cap it was chosen under, an agent slot free when it needs one). `started`: \
             `ticketId`, `key`, `attemptId`; `skipped`: none was free (or the disk is almost full); `failed`: a start \
             failed, and that ticket is passed over until it changes.",
            &[("started", "Started"), ("skipped", "Skipped"), ("failed", "Failed")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: StartConfig = parse(config)?;
        c.fields().map(|_| ())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: StartConfig = parse(&input)?;
            let fields = c.fields()?;
            let tickets = c.tickets()?;
            let per_project = c.per_project.unwrap_or(1).max(1);
            if cx.dry_run {
                return Ok(Outcome::next("started", json!({"would": dry_run(cx.st, &tickets, per_project).await})));
            }
            // A disk almost full: nothing new starts until space is freed (PDT-324).
            if crate::storage::auto_start_paused(cx.st).await {
                return Ok(Outcome::next("skipped", json!({"reason": "the disk is almost full", "quiet": true})));
            }
            let host = cx.st.store.get_host_settings().await.map_err(|e| e.to_string())?;
            let _held = cx.st.pickup_lock.lock().await;
            // Run again after a restart: the start it was making when PDT stopped counts if it got its attempt.
            let trying =
                cx.rerun.then(|| cx.step.output.get(TRYING).and_then(Value::as_str).map(String::from)).flatten();
            let (mut started, mut skipped, mut failed) = (vec![], vec![], vec![]);
            let mut per: HashMap<String, u32> = HashMap::new();
            for (id, caps) in tickets {
                let Some(t) = cx.st.store.get_ticket_summary(&id).await.map_err(|e| e.to_string())? else {
                    skipped.push(json!({"ticketId": id, "reason": "no such ticket"}));
                    continue;
                };
                let note = |reason: &str| json!({"ticketId": t.id, "key": t.key, "reason": reason});
                if per.get(&t.project_id).is_some_and(|n| *n >= per_project) {
                    continue;
                }
                if let Some(a) = t.attempt.as_ref().filter(|a| a.state == AttemptState::Active) {
                    if trying.as_deref() == Some(t.id.as_str()) {
                        *per.entry(t.project_id.clone()).or_default() += 1;
                        started.push(json!({"ticketId": t.id, "key": t.key, "attemptId": a.id}));
                    } else {
                        skipped.push(note("already started"));
                    }
                    continue;
                }
                // The caps it was chosen under, again now that no other pickup start runs: another run may have
                // started a ticket since this one chose.
                if let Some(caps) = &caps {
                    let in_flight = in_flight(cx.st, &t.project_id).await?;
                    let active = u32::try_from(in_flight.len()).unwrap_or(u32::MAX);
                    if active >= caps.cap {
                        skipped.push(note(&format!("the project is at its cap ({active} of {} in flight)", caps.cap)));
                        continue;
                    }
                    let pick = pdt_core::pickup::Pick {
                        ticket: PickupTicket::from(&t),
                        cap: caps.cap,
                        area_caps: caps.area_caps.clone(),
                        label_caps: caps.label_caps.clone(),
                        overlap_rule: caps.overlap_rule.clone(),
                    };
                    if let Some((what, name, cap)) = pick.full_at(&in_flight) {
                        skipped.push(note(&format!("{what} {name} is at its cap of {cap}")));
                        continue;
                    }
                    if let Some(reason) = caps.overlap_rule.check(&t.areas, &in_flight) {
                        skipped.push(note(&reason.describe()));
                        continue;
                    }
                }
                let no_session = match c.hand_to {
                    HandTo::Auto => {
                        crate::scheduler::workflow_listens(
                            cx.st,
                            &t.project_id,
                            pdt_core::workflow::events::ATTEMPT_STARTED,
                        )
                        .await
                    }
                    HandTo::Session => false,
                    HandTo::Workflow => true,
                };
                // A start with its own session runs a turn at once: it waits for a slot like an agent step.
                if !no_session && !crate::scheduler::slot_free(cx.st).await {
                    skipped.push(note("no agent slot is free"));
                    continue;
                }
                let input = StartTicket {
                    agent: fields.agent.unwrap_or(host.default_agent),
                    branch: None,
                    base: None,
                    bypass_permissions: None,
                    permission_mode: Some(fields.permission_mode.unwrap_or(host.ticket_permission_mode)),
                    tab: None,
                    new_workspace: Some(true),
                    mode: Some(StartMode::Check),
                    session_mode: Some(fields.session_mode.unwrap_or(host.default_session_mode)),
                    from_session: None,
                    no_session: Some(no_session),
                    // A parent (candidates kept parents) starts its next part.
                    next: Some(true),
                    cols: None,
                    rows: None,
                };
                let mut v = input.validate().map_err(|e| e.to_string())?;
                // Checked just now, with the cap it was chosen under: a step's cap above the project's running cap
                // holds (Start still refuses for blockers and overlaps).
                v.cap_checked = caps.is_some();
                cx.save_progress(&json!({TRYING: t.id})).await?;
                match crate::start::start_ticket(cx.st, t.id.clone(), v, Starter::Auto).await {
                    Ok(r) => {
                        tracing::info!(ticket = %t.key, workflow = %cx.workflow.name, "a workflow started a ticket");
                        *per.entry(t.project_id.clone()).or_default() += 1;
                        started.push(json!({"ticketId": r.ticket.summary.id, "key": r.ticket.summary.key,
                                            "attemptId": r.attempt.id}));
                    }
                    // Started meanwhile (by the user, say): not this ticket's failure.
                    Err(e) if e.code() == "ticket_active" => skipped.push(note("already started")),
                    Err(e) => {
                        let message = e.message().to_string();
                        // A failed write (a full disk) is tried again once writes work, not passed over (PDT-323).
                        let transient = pdt_store::is_write_failure(&message);
                        if !transient {
                            tracing::warn!(ticket = %t.key, error = %message, "could not start a ticket; passing it over until it changes");
                            cx.st
                                .auto_start_failed
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(t.id.clone(), t.updated_at);
                        }
                        failed.push(json!({"ticketId": t.id, "key": t.key, "error": message, "transient": transient}));
                    }
                }
            }
            let mut out = json!({"started": started, "skipped": skipped, "failed": failed});
            let port = match started.first() {
                Some(first) => {
                    for k in ["ticketId", "key", "attemptId"] {
                        out[k] = first[k].clone();
                    }
                    "started"
                }
                None if !failed.is_empty() => "failed",
                None => {
                    out["quiet"] = json!(true);
                    "skipped"
                }
            };
            Ok(Outcome::Next { port: port.into(), output: out })
        })
    }

    fn default_timeout_ms(&self) -> u64 {
        5 * 60 * 1000
    }
}

/// What a dry run would start: the first ticket of each project.
async fn dry_run(st: &AppState, tickets: &[(String, Option<Caps>)], per_project: u32) -> Vec<String> {
    let mut per: HashMap<String, u32> = HashMap::new();
    let mut out = vec![];
    for (id, _) in tickets {
        let Ok(Some(t)) = st.store.get_ticket_summary(id).await else { continue };
        let n = per.entry(t.project_id.clone()).or_default();
        if *n < per_project {
            *n += 1;
            out.push(format!("start {}", t.key));
        }
    }
    out
}
