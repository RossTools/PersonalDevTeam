//! The built-in node kinds, one module each (Phase 7a spec §4.4).

pub mod agent;
pub mod attempt;
pub mod brief;
pub mod call;
pub mod command;
pub mod condition;
pub mod end;
pub mod gate;
pub mod github;
pub mod http;
pub mod inbox_admit;
pub mod inbox_notice;
pub mod mcp_tool;
pub mod merge;
pub mod notify;
pub mod outside_pr;
pub mod release;
pub mod schedule;
pub mod session_send;
pub mod state;
pub mod switch;
pub mod ticket;
pub mod trigger;
pub mod wait;
pub mod webhook;

/// Sets the run's ticket's pipeline stage (the Board's column) when a step that has one starts.
pub(crate) async fn set_stage(
    cx: &crate::workflow::catalog::StepCx<'_>,
    stage: Option<pdt_core::ticket::PipelineStage>,
) {
    if let (Some(stage), Some(ticket)) = (stage, &cx.run.ticket_id)
        && let Err(e) = cx.st.store.set_pipeline_stage(ticket, Some(stage)).await
    {
        tracing::warn!(ticket = %ticket, error = %e, "could not set the ticket's pipeline stage");
    }
}
