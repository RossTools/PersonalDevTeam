//! The board's settings for a project on this host (PDT-377): how far back its Done column (and its closed parents'
//! lanes) reaches.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The Done windows the board offers, in days.
pub const DONE_WINDOW_DAYS: &[u32] = &[1, 3, 7, 14, 30];

/// The Done window a project starts with, in days.
pub const DEFAULT_DONE_WINDOW_DAYS: u32 = 14;

/// A project's board settings (`GET`/`PUT /api/projects/{id}/board`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BoardSettings {
    /// The Done column shows tickets closed in the last this many days: one of 1, 3, 7, 14 or 30.
    #[serde(default = "default_done_window")]
    pub done_window_days: u32,
}

fn default_done_window() -> u32 {
    DEFAULT_DONE_WINDOW_DAYS
}

impl Default for BoardSettings {
    fn default() -> Self {
        BoardSettings { done_window_days: DEFAULT_DONE_WINDOW_DAYS }
    }
}

impl BoardSettings {
    pub fn validate(self) -> Result<Self, String> {
        if !DONE_WINDOW_DAYS.contains(&self.done_window_days) {
            let days: Vec<String> = DONE_WINDOW_DAYS.iter().map(u32::to_string).collect();
            return Err(format!("the Done window is one of {} days", days.join(", ")));
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_done_window_is_one_of_the_choices() {
        assert_eq!(BoardSettings::default().done_window_days, 14);
        assert_eq!(serde_json::from_str::<BoardSettings>("{}").unwrap(), BoardSettings::default());
        for d in DONE_WINDOW_DAYS {
            assert!(BoardSettings { done_window_days: *d }.validate().is_ok());
        }
        let e = BoardSettings { done_window_days: 5 }.validate().unwrap_err();
        assert_eq!(e, "the Done window is one of 1, 3, 7, 14, 30 days");
        assert!(BoardSettings { done_window_days: 0 }.validate().is_err());
    }
}
