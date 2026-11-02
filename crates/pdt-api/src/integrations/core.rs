//! PDT's own pieces, registered like any integration: tickets, attempts, pull requests and CI events, the flow,
//! ticket, session and agent node kinds, and the pdt-mcp tools that work on PDT itself.

use super::{Integration, Tool};
use crate::mcp::tools::Cx;
use crate::workflow::catalog::{BoxFut, NodeKind};
use pdt_core::pm::AskTarget;
use pdt_core::workflow::events::{EVENT_TYPES, EventType};
use serde_json::Value;

pub struct Core;

impl Integration for Core {
    fn id(&self) -> &'static str {
        "core"
    }

    fn event_types(&self) -> &'static [EventType] {
        EVENT_TYPES
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        use crate::workflow::nodes::*;
        vec![
            Box::new(trigger::Manual),
            Box::new(schedule::ScheduleTrigger),
            Box::new(condition::ConditionKind),
            Box::new(switch::Switch),
            Box::new(gate::Gate),
            Box::new(wait::Wait),
            Box::new(call::Call),
            Box::new(end::End),
            Box::new(state::StateSet),
            Box::new(session_send::SessionSend),
            Box::new(ticket::Finish),
            Box::new(ticket::Move),
            Box::new(ticket::Comment),
            Box::new(ticket::Create),
            Box::new(parent::CloseParent),
            Box::new(pickup::Candidates),
            Box::new(pickup::Choose),
            Box::new(pickup::Start),
            Box::new(agent::Agent),
            Box::new(agent_stuck::Nudge),
            Box::new(agent_stuck::Restart),
            Box::new(agent_stuck::Ask),
            Box::new(agent_stuck::Stop),
            Box::new(command::Command),
            Box::new(notify::Notify),
            Box::new(inbox_notice::InboxNotice),
            Box::new(inbox_admit::InboxAdmit),
            Box::new(ticket_proposal::TicketProposal),
            Box::new(brief::Archive),
            Box::new(brief::Read),
            Box::new(pm::PmUpdate),
            Box::new(pm::PmCheckpoint),
            Box::new(merge::Merge),
            Box::new(attempt::AgentWorking),
            Box::new(attempt::HasChanges),
            Box::new(conflicts::DeferConflict),
            Box::new(conflicts::UpdateBase),
        ]
    }

    fn tools(&self) -> Vec<Tool> {
        crate::mcp::tools::defs()
    }

    fn call_tool<'a>(&'a self, cx: &'a Cx<'a>, name: &'a str, args: Value) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(crate::mcp::tools::call(cx, name, args))
    }

    fn ask_target(&self, tool: &str, input: &Value) -> Option<AskTarget> {
        pdt_core::pm::ask_target(tool, input)
    }
}
