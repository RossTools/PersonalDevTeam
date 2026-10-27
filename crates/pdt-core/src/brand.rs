//! The single source of product naming. A rename touches this file, desktop/src-tauri/tauri.conf.json
//! (productName, identifier), desktop/src-tauri/tauri.bundle.conf.json (sidecars),
//! desktop/src-tauri/windows/hooks.nsh (pdt-host.exe) and scripts/release-stable.* (product name, slug, port) only,
//! and HOST_* / TRAY_* below. The release repository is RELEASE_REPO below.

pub const PRODUCT_NAME: &str = "Personal Dev Team";
pub const SLUG: &str = "pdt";
/// The short name agents and prompts use for the product (ticket prompts: "Ticket PDT-4 in PDT").
pub const SHORT_NAME: &str = "PDT";
pub const DAEMON_BIN: &str = "pdtd";
pub const STABLE_PORT: u16 = 7420;
pub const DEV_PORT: u16 = 7421;
/// The supervisor that runs on each host: starts pdtd, installs updates, rolls back (remote-host spec §3.1).
pub const HOST_BIN: &str = "pdt-host";
/// launchd label of the per-user LaunchAgent that runs pdt-host (macOS).
pub const HOST_LABEL: &str = "dev.personaldevteam.host";
/// Name of the per-user autostart entry for pdt-host (Windows `HKCU\…\Run` value).
pub const HOST_AUTOSTART_NAME: &str = "Personal Dev Team host";
/// launchd label of the per-user LaunchAgent that starts the tray app at login (macOS, PDT-148).
pub const TRAY_LABEL: &str = "dev.personaldevteam.tray";
/// Name of the per-user autostart entry for the tray app (Windows `HKCU\…\Run` value, PDT-148).
pub const TRAY_AUTOSTART_NAME: &str = "Personal Dev Team";
/// The app bundle's identifier (tauri.conf.json `identifier`, a test checks). pdt-host's LaunchAgent names it in
/// `AssociatedBundleIdentifiers`, so macOS counts pdt-host and pdtd as the app's (Local Network permission, PDT-156).
pub const APP_BUNDLE_ID: &str = "dev.personaldevteam.desktop";
/// GitHub repository (owner/name) whose releases hosts update from, through the GitHub REST API and a token.
pub const RELEASE_REPO: &str = "RossTools/PersonalDevTeam";
/// Service name under which secrets (the update token) are stored in the OS keychain / credential manager.
pub const KEYCHAIN_SERVICE: &str = "dev.personaldevteam";
/// The client ID of PDT's GitHub OAuth App (PDT-177): the in-app "Connect GitHub" login (OAuth device flow) uses it.
/// Registered under the RossTools organization with Device Flow enabled (docs/reference/github.md); not a secret.
pub const GITHUB_OAUTH_CLIENT_ID: &str = "Ov23litfc05CvYvo8kR5";
/// The optional run configuration file inside a project's repository (remote-host spec §4.1).
pub const PROJECT_CONFIG_FILE: &str = ".pdt/project.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Profile {
    Stable,
    Dev,
}

impl Profile {
    /// Interprets the `PDT_PROFILE` environment value. Only the exact string `dev` selects Dev.
    pub fn from_env_value(value: Option<&str>) -> Profile {
        match value {
            Some("dev") => Profile::Dev,
            _ => Profile::Stable,
        }
    }

    pub fn port(self) -> u16 {
        match self {
            Profile::Stable => STABLE_PORT,
            Profile::Dev => DEV_PORT,
        }
    }

    pub fn data_dir_name(self) -> String {
        match self {
            Profile::Stable => SLUG.to_string(),
            Profile::Dev => format!("{SLUG}-dev"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_bundle_id_is_the_tauri_identifier() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../../desktop/src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], APP_BUNDLE_ID);
        assert_eq!(conf["productName"], PRODUCT_NAME);
    }

    #[test]
    fn dev_profile_only_from_exact_value() {
        assert_eq!(Profile::from_env_value(Some("dev")), Profile::Dev);
        assert_eq!(Profile::from_env_value(Some("DEV")), Profile::Stable);
        assert_eq!(Profile::from_env_value(Some("")), Profile::Stable);
        assert_eq!(Profile::from_env_value(None), Profile::Stable);
    }

    #[test]
    fn profiles_have_distinct_ports_and_dirs() {
        assert_eq!(Profile::Stable.port(), 7420);
        assert_eq!(Profile::Dev.port(), 7421);
        assert_eq!(Profile::Stable.data_dir_name(), "pdt");
        assert_eq!(Profile::Dev.data_dir_name(), "pdt-dev");
    }
}
