//! Best-effort writes whose failure the user would otherwise never hear of: logged, not dropped with `let _ =`.

use std::fmt::Display;

pub(crate) trait LogErr {
    /// Logs a failure as a warning: "could not {what}".
    fn log_warn(self, what: &str);
}

impl<T, E: Display> LogErr for Result<T, E> {
    fn log_warn(self, what: &str) {
        if let Err(e) = self {
            tracing::warn!(error = %e, "could not {what}");
        }
    }
}
