//! Settings → Agents → When an agent is stuck (PDT-367): the built-in workflow's inputs, through
//! [`crate::builtin_settings`]. Its reset leaves the workflow on or off: Settings has a switch for that.

use crate::builtin_settings::{Reset, router_with};
use crate::routes::AppState;
use crate::workflow::builtins::stuck_agents::SETTINGS;
use axum::Router;

pub fn router() -> Router<AppState> {
    router_with("/stuck-agents", &SETTINGS, Reset { turns_on: false, then: None })
}
