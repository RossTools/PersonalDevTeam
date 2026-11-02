//! Settings → This host → PM (PDT-384): the PM daily checkpoint's inputs (a time of day, the days, a chat channel to
//! post the daily update to), through [`crate::builtin_settings`].

use crate::routes::AppState;
use crate::workflow::builtins::pm_daily_checkpoint::SETTINGS;
use axum::Router;

pub fn router() -> Router<AppState> {
    crate::builtin_settings::router("/pm-checkpoint", &SETTINGS)
}
