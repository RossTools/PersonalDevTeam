//! Which instance of PDT this daemon is (stable or a development one), reported by `/api/health`.

use crate::brand::{Profile, STABLE_PORT};
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceInfo {
    pub profile: Profile,
    pub port: u16,
    /// A development instance: the dev profile, or any port other than the stable one (e.g. a "pdt" run).
    pub dev: bool,
}

impl InstanceInfo {
    pub fn new(profile: Profile, port: u16) -> Self {
        Self { profile, port, dev: profile == Profile::Dev || port != STABLE_PORT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brand::DEV_PORT;

    #[test]
    fn stable_profile_on_stable_port_is_not_dev() {
        assert!(!InstanceInfo::new(Profile::Stable, STABLE_PORT).dev);
    }

    #[test]
    fn dev_profile_is_dev_on_any_port() {
        assert!(InstanceInfo::new(Profile::Dev, DEV_PORT).dev);
        assert!(InstanceInfo::new(Profile::Dev, STABLE_PORT).dev);
    }

    #[test]
    fn stable_profile_on_another_port_is_dev() {
        let info = InstanceInfo::new(Profile::Stable, 8512);
        assert!(info.dev);
        assert_eq!((info.profile, info.port), (Profile::Stable, 8512));
    }

    #[test]
    fn serializes_profile_lowercase() {
        let v = serde_json::to_value(InstanceInfo::new(Profile::Dev, 8512)).unwrap();
        assert_eq!(v, serde_json::json!({"profile": "dev", "port": 8512, "dev": true}));
    }
}
