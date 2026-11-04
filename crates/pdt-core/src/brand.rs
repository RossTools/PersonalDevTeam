//! The single source of product naming. A rename touches this file, desktop/src-tauri/tauri.conf.json
//! (productName, identifier), desktop/src-tauri/tauri.bundle.conf.json (sidecars),
//! desktop/src-tauri/windows/hooks.nsh (pdt-host.exe), scripts/release-stable.* (product name, slug, port) and the
//! one-line install scripts site/public/install.{sh,ps1} (product name, slug, port, release repository; a test checks),
//! site/src/links.mjs (port; a test checks) only, and HOST_* / TRAY_* below. The release repository is RELEASE_REPO below.

pub const PRODUCT_NAME: &str = "Personal Dev Team";
pub const SLUG: &str = "pdt";
/// The short name agents and prompts use for the product (ticket prompts: "Ticket PDT-4 in PDT").
pub const SHORT_NAME: &str = "PDT";
pub const DAEMON_BIN: &str = "pdtd";
pub const STABLE_PORT: u16 = 7420;
pub const DEV_PORT: u16 = 7421;
/// The supervisor that runs on each host: starts pdtd, installs updates, rolls back.
pub const HOST_BIN: &str = "pdt-host";
/// launchd label of the per-user LaunchAgent that runs pdt-host (macOS).
pub const HOST_LABEL: &str = "dev.personaldevteam.host";
/// Name of the per-user autostart entry for pdt-host (Windows `HKCU\…\Run` value).
pub const HOST_AUTOSTART_NAME: &str = "Personal Dev Team host";
/// The systemd user unit that runs pdt-host (Linux): `~/.config/systemd/user/<this>`.
pub const HOST_SYSTEMD_UNIT: &str = "pdt-host.service";
/// launchd label of the per-user LaunchAgent that starts the tray app at login (macOS, PDT-148).
pub const TRAY_LABEL: &str = "dev.personaldevteam.tray";
/// Name of the per-user autostart entry for the tray app (Windows `HKCU\…\Run` value, PDT-148).
pub const TRAY_AUTOSTART_NAME: &str = "Personal Dev Team";
/// The app bundle's identifier (tauri.conf.json `identifier`, a test checks). pdt-host's LaunchAgent names it in
/// `AssociatedBundleIdentifiers`, so macOS counts pdt-host and pdtd as the app's (Local Network permission, PDT-156).
pub const APP_BUNDLE_ID: &str = "dev.personaldevteam.desktop";
/// GitHub repository (owner/name) whose releases hosts update from, through the GitHub REST API and a token.
pub const RELEASE_REPO: &str = "RossTools/PersonalDevTeam";
/// RELEASE_REPO's numeric GitHub id. Updates and bug reports reach the repository by it (`/repositories/{id}`), which
/// survives renames, org renames and transfers (PDT-195); the name is the fallback. Renaming or moving the repository:
/// docs/SDLC.md "Moving the release repository".
pub const RELEASE_REPO_ID: u64 = 1420061987;
/// The public key that signs each release's SHA256SUMS (PDT-531; docs/SDLC.md "Release signing"): base64 of its
/// SubjectPublicKeyInfo DER, an ECDSA P-256 key. With a key, updates and the install scripts refuse a release whose
/// `SHA256SUMS.sig` doesn't verify; None trusts SHA256SUMS alone (until the release key exists). The install scripts
/// pin the same value (a test checks).
pub const RELEASE_SIGNING_KEY: Option<&str> = None;
/// The Apple Developer Team ID that signs the macOS app. pdt-host installs an update's `.app` only when
/// `codesign` verifies it as signed by this team.
pub const MACOS_TEAM_ID: &str = "DX69RXFP5X";
/// Service name under which secrets (the update token) are stored in the OS keychain / credential manager.
pub const KEYCHAIN_SERVICE: &str = "dev.personaldevteam";
/// The client ID of PDT's GitHub OAuth App (PDT-177): the in-app "Connect GitHub" login (OAuth device flow) uses it.
/// Registered under the RossTools organization with Device Flow enabled (docs/dev/architecture.md "GitHub"); not a secret.
pub const GITHUB_OAUTH_CLIENT_ID: &str = "Ov23litfc05CvYvo8kR5";
/// The project's website (`site/`). Google's sign-in comes back through its `/oauth/google` page (PDT-360).
pub const WEBSITE: &str = "https://pdt.dev";
/// The optional run configuration file inside a project's repository.
pub const PROJECT_CONFIG_FILE: &str = ".pdt/project.toml";

/// The User-Agent of PDT's HTTP clients: `pdt/<build version>`, or `pdt-<component>/<build version>` for one part
/// (`workflows`, `updater`). Every client names the build, so a service's logs say which PDT called.
pub fn user_agent(component: &str) -> String {
    match component {
        "" => format!("{SLUG}/{}", crate::BUILD_VERSION),
        c => format!("{SLUG}-{c}/{}", crate::BUILD_VERSION),
    }
}

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

    /// PDT never special-cases its own repository (PDT-295; docs/dev/invariants.md): outside tests, only these name
    /// the release repository, PDT as the vendor (updates, bug reports, the release repository setting).
    const RELEASE_REPO_USERS: &[(&str, &str)] = &[
        ("crates/pdt-core/src/brand.rs", "where it is defined"),
        ("crates/pdt-core/src/host.rs", "the release repository setting (its override)"),
        ("crates/pdt-api/src/update.rs", "updates and bug reports"),
        ("crates/pdt-api/src/routes.rs", "updates and bug reports (the default update service)"),
        ("crates/pdtd/src/main.rs", "updates and bug reports (pdtd's update service)"),
        ("crates/pdt-int-discord/src/bot.rs", "Discord's User-Agent names the product's home"),
    ];

    /// The source files of the product (tests left out: `tests/`, `tests.rs`, a file's `#[cfg(test)]` part).
    fn product_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !["tests", "node_modules", "target", "gen"].contains(&name.as_str()) {
                    product_sources(&path, out);
                }
            } else if (name.ends_with(".rs") && name != "tests.rs")
                || ((name.ends_with(".ts") || name.ends_with(".tsx")) && !name.contains(".test."))
            {
                out.push(path);
            }
        }
    }

    /// Whether `text` names the release repository: the constants (not `PDT_RELEASE_REPO`, the override's variable),
    /// or the repository itself.
    fn names_release_repo(text: &str) -> bool {
        let constant = text
            .match_indices("RELEASE_REPO")
            .any(|(i, _)| !text[..i].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_'));
        constant || text.to_ascii_lowercase().contains(&RELEASE_REPO.to_ascii_lowercase())
    }

    #[test]
    fn only_updates_and_bug_reports_name_the_release_repository() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = vec![];
        for dir in ["crates", "desktop/src-tauri/src", "web/src", "web/ui"] {
            product_sources(&root.join(dir), &mut files);
        }
        let crates = files.iter().filter(|f| f.extension().is_some_and(|e| e == "rs")).count();
        assert!(crates > 100, "found the sources ({crates})");
        let mut found = vec![];
        for f in &files {
            let text = std::fs::read_to_string(f).unwrap();
            let product = text.split("#[cfg(test)]").next().unwrap_or_default();
            let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            if names_release_repo(product) && !RELEASE_REPO_USERS.iter().any(|(p, _)| *p == rel) {
                found.push(rel);
            }
        }
        assert!(found.is_empty(), "name the release repository outside updates and bug reports: {found:?}");
        assert!(names_release_repo("brand::RELEASE_REPO_ID") && names_release_repo("\"rosstools/personaldevteam\""));
        assert!(!names_release_repo("PDT_RELEASE_REPO"));
    }

    #[test]
    fn the_app_bundle_id_is_the_tauri_identifier() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../../desktop/src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], APP_BUNDLE_ID);
        assert_eq!(conf["productName"], PRODUCT_NAME);
    }

    #[test]
    fn the_install_scripts_use_these_names() {
        let sh = include_str!("../../../site/public/install.sh");
        for line in [
            format!("repo=\"{RELEASE_REPO}\""),
            format!("product=\"{PRODUCT_NAME}\""),
            format!("slug=\"{SLUG}\""),
            format!("port={STABLE_PORT}"),
            format!("signing_key=\"{}\"", RELEASE_SIGNING_KEY.unwrap_or_default()),
        ] {
            assert!(sh.contains(&line), "install.sh lacks {line}");
        }
        assert!(HOST_BIN == format!("{SLUG}-host"), "install.sh runs $slug-host");
        let ps1 = include_str!("../../../site/public/install.ps1");
        for line in [
            format!("$repo = '{RELEASE_REPO}'"),
            format!("$product = '{PRODUCT_NAME}'"),
            format!("$port = {STABLE_PORT}"),
            format!("'{HOST_BIN}.exe'"),
            format!("$signingKey = '{}'", RELEASE_SIGNING_KEY.unwrap_or_default()),
        ] {
            assert!(ps1.contains(&line), "install.ps1 lacks {line}");
        }
        let links = include_str!("../../../site/src/links.mjs");
        assert!(links.contains(&format!("export const STABLE_PORT = {STABLE_PORT};")), "links.mjs lacks the port");
    }

    #[test]
    fn user_agents_name_the_build() {
        assert_eq!(user_agent(""), format!("pdt/{}", crate::BUILD_VERSION));
        assert_eq!(user_agent("updater"), format!("pdt-updater/{}", crate::BUILD_VERSION));
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
