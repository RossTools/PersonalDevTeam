//! The `schedule` trigger (Phase 7e spec §2.2) and the scheduler that emits its `schedule.fired` events: one per slot
//! (keyed, so a slot fires once), a baseline for a new or changed workflow, and only the latest missed slot after
//! pdtd was down (with `catchUp`).

use crate::routes::{AppState, now_ms};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, ports, schema_of};
use pdt_core::workflow::api::{LogEvent, NewLogEvent, Workflow};
use pdt_core::workflow::cond::Condition;
use pdt_core::workflow::doc::Node;
use pdt_core::workflow::events::{SCHEDULE_FIRED, ScheduleFired, event_type};
use pdt_core::workflow::schedule::{Mark, Schedule, decide};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const SCHEDULE: &str = "schedule";

/// Workflow state keys starting with this belong to PDT (`{{state}}` doesn't show them).
pub const RESERVED_STATE: &str = "_pdt.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct ScheduleConfig {
    /// An interval: `"15m"`, `"1h"`, `"1d"` (at least a minute), aligned to midnight UTC.
    pub every: Option<String>,
    /// A 5-field cron expression (minute hour day month weekday): `"0 7 * * 1-5"`.
    pub cron: Option<String>,
    /// The cron expression's time zone (IANA, `"Europe/London"`); default the host's.
    pub timezone: Option<String>,
    /// After PDT was down, run once for the latest missed time.
    pub catch_up: bool,
    /// Only slots for which every condition holds.
    #[serde(rename = "where")]
    #[ts(rename = "where")]
    pub conditions: Vec<Condition>,
    /// The run's idempotency key (`"brief:{{trigger.date}}"`); default: the slot.
    pub run_key: Option<String>,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self { every: None, cron: None, timezone: None, catch_up: true, conditions: vec![], run_key: None }
    }
}

impl ScheduleConfig {
    pub fn schedule(&self) -> Result<Schedule, String> {
        Schedule::parse(self.every.as_deref(), self.cron.as_deref(), self.timezone.as_deref())
    }
}

pub struct ScheduleTrigger;

impl NodeKind for ScheduleTrigger {
    fn spec(&self) -> KindSpec {
        let t = event_type(SCHEDULE_FIRED).expect("schedule.fired is an event type");
        KindSpec {
            id: SCHEDULE.into(),
            version: 1,
            category: Category::Trigger,
            title: t.title.into(),
            description: "Runs every interval (`every`) or at the times of a cron expression (`cron`, in `timezone`). \
                          A new or changed schedule first runs at its next time."
                .into(),
            config_schema: schema_of::<ScheduleConfig>(),
            output_schema: serde_json::to_value((t.schema)()).unwrap_or(Value::Null),
            ports: ports(&[("out", "Then")]),
            dynamic_ports: false,
            preset: None,
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<ScheduleConfig>(config)?.schedule().map(|_| ())
    }

    fn event_type(&self) -> Option<&str> {
        Some(SCHEDULE_FIRED)
    }

    fn accepts(&self, _config: &Value, wf: &Workflow, node: &Node, event: &LogEvent) -> bool {
        let s = |k: &str| event.payload.get(k).and_then(|v| v.as_str());
        s("workflowId") == Some(wf.id.as_str()) && s("nodeId") == Some(node.id.as_str())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, _input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move { Ok(Outcome::Next { port: "out".into(), output: cx.run.trigger.clone() }) })
    }
}

/// The state key of a schedule node's mark.
fn mark_key(node: &str) -> String {
    format!("{RESERVED_STATE}schedule.{node}")
}

/// Emits `schedule.fired` for every enabled schedule whose slot came (engine tick, before matching events).
pub(crate) async fn fire_schedules(st: &AppState) {
    let now = now_ms();
    let enabled = match crate::workflow::engine::enabled_with_docs(st).await {
        Ok(w) => w,
        Err(e) => return tracing::warn!(error = %e, "could not read the enabled workflows"),
    };
    for (wf, doc) in enabled {
        let nodes: Vec<&Node> = doc.nodes.iter().filter(|n| n.kind == SCHEDULE).collect();
        if nodes.is_empty() {
            continue;
        }
        let state = st.store.workflow_state(&wf.id).await.unwrap_or_default();
        for node in nodes {
            let Ok(cfg) = st.registry().catalog().current_config(node).and_then(|c| parse::<ScheduleConfig>(&c)) else {
                continue;
            };
            let Ok(schedule) = cfg.schedule() else { continue };
            let key = mark_key(&node.id);
            let prev: Option<Mark> = state.get(&key).and_then(|v| serde_json::from_value(v.clone()).ok());
            let (fire, mark) = decide(prev, schedule.latest_slot(now), wf.updated_at, cfg.catch_up, now);
            if let Some(slot) = fire {
                let (date, slot_iso) = schedule.describe_slot(slot);
                let payload =
                    ScheduleFired { workflow_id: wf.id.clone(), node_id: node.id.clone(), slot, slot_iso, date };
                let mut e = NewLogEvent::new(
                    SCHEDULE_FIRED,
                    format!("schedule:{}:{}:{slot}", wf.id, node.id),
                    "scheduler",
                    payload,
                );
                e.project_id = wf.project_id.clone();
                if let Err(x) = st.store.emit_event(&e, now).await {
                    tracing::warn!(workflow = %wf.name, error = %x, "could not record a schedule");
                    continue;
                }
                tracing::info!(workflow = %wf.name, node = %node.id, slot, "schedule fired");
            }
            if let Some(m) = mark {
                let v = serde_json::to_value(m).unwrap_or(Value::Null);
                if let Err(x) = st.store.set_workflow_state(&wf.id, &key, &v, false, now).await {
                    tracing::warn!(workflow = %wf.name, error = %x, "could not record a schedule's slot");
                }
            }
        }
    }
}
