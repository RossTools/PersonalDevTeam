//! Thin wrapper over the `tailscale` CLI (verified fact 5). Blocking; call from `spawn_blocking` in async code.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsStatus {
    /// `Self.DNSName` without its trailing dot, e.g. `box.tail1234.ts.net`.
    pub dns_name: String,
    /// The login of the user this machine is logged in as (offered as the host owner).
    pub self_login: Option<String>,
    /// HTTPS certificates are enabled for the tailnet (`CertDomains` is non-empty); `serve --https` needs them.
    pub https_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TsError {
    #[error("tailscale CLI not found")]
    NotInstalled,
    #[error("Tailscale is not running ({0})")]
    NotRunning(String),
    #[error("HTTPS certificates are not enabled for this tailnet (Tailscale admin console → DNS → Enable HTTPS)")]
    HttpsDisabled,
    #[error("tailscale failed: {0}")]
    Command(String),
    #[error("unexpected tailscale status output: {0}")]
    BadStatus(String),
}

pub trait TailscaleApi: Send + Sync {
    fn status(&self) -> Result<TsStatus, TsError>;
    /// Maps `https://<machine>:<https_port>/` to `http://127.0.0.1:<target_port>` in the background (persists).
    fn serve(&self, https_port: u16, target_port: u16) -> Result<(), TsError>;
    /// Removes the mapping on `https_port`.
    fn unserve(&self, https_port: u16) -> Result<(), TsError>;
}

pub fn serve_args(https_port: u16, target_port: u16) -> Vec<String> {
    vec!["serve".into(), "--bg".into(), format!("--https={https_port}"), format!("http://127.0.0.1:{target_port}")]
}

/// "add `off` to the end … the target is optional, but all original flags are required" (fact 5).
pub fn unserve_args(https_port: u16) -> Vec<String> {
    vec!["serve".into(), format!("--https={https_port}"), "off".into()]
}

pub fn parse_status(json: &str) -> Result<TsStatus, TsError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| TsError::BadStatus(e.to_string()))?;
    let state = v["BackendState"].as_str().unwrap_or("");
    if state != "Running" {
        return Err(TsError::NotRunning(format!("state {state}")));
    }
    let dns = v["Self"]["DNSName"].as_str().unwrap_or("").trim_end_matches('.').to_string();
    if dns.is_empty() {
        return Err(TsError::BadStatus("Self.DNSName is empty (is MagicDNS enabled?)".into()));
    }
    let self_login = match &v["Self"]["UserID"] {
        serde_json::Value::Null => None,
        id => {
            let key = id.as_u64().map(|n| n.to_string()).or_else(|| id.as_str().map(String::from));
            key.and_then(|k| v["User"][&k]["LoginName"].as_str().map(String::from))
        }
    };
    let https_enabled = v["CertDomains"].as_array().is_some_and(|a| !a.is_empty());
    Ok(TsStatus { dns_name: dns, self_login, https_enabled })
}

fn classify_failure(stderr: &str) -> TsError {
    let lower = stderr.to_lowercase();
    if lower.contains("failed to connect") || lower.contains("not running") || lower.contains("is stopped") {
        TsError::NotRunning(stderr.trim().to_string())
    } else {
        TsError::Command(stderr.trim().to_string())
    }
}

/// The real CLI. Only production wiring (pdtd) detects one; tests use [`testing::RecordingTailscale`], or the
/// `fake-tailscale` binary through [`TailscaleCli::new`].
pub struct TailscaleCli {
    program: Option<PathBuf>,
    env: Vec<(String, String)>,
}

impl TailscaleCli {
    pub fn new(program: PathBuf) -> Self {
        Self { program: Some(program), env: Vec::new() }
    }

    /// Finds the CLI on `login_path` (or pdtd's PATH), then in the standard install locations.
    pub fn detect(login_path: Option<&str>) -> Self {
        let paths = login_path.map(std::ffi::OsString::from).or_else(|| std::env::var_os("PATH"));
        let on_path = std::env::current_dir().ok().and_then(|cwd| which::which_in("tailscale", paths, cwd).ok());
        let program = on_path.or_else(|| candidates().into_iter().find(|p| p.is_file()));
        Self { program, env: Vec::new() }
    }

    /// Sets a variable for every invocation (tests point the fake CLI at its files this way).
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    fn run(&self, args: &[String]) -> Result<String, TsError> {
        let program = self.program.as_ref().ok_or(TsError::NotInstalled)?;
        let mut cmd = Command::new(program);
        cmd.args(args).envs(self.env.iter().map(|(k, v)| (k, v)));
        no_window(&mut cmd);
        let out = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TsError::NotInstalled
            } else {
                TsError::Command(e.to_string())
            }
        })?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let mut err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if err.is_empty() {
                err = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
            Err(classify_failure(&err))
        }
    }
}

fn candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe")]
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
            PathBuf::from("/opt/homebrew/bin/tailscale"),
            PathBuf::from("/usr/local/bin/tailscale"),
        ]
    } else {
        vec![PathBuf::from("/usr/bin/tailscale")]
    }
}

fn no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        _cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

impl TailscaleApi for TailscaleCli {
    fn status(&self) -> Result<TsStatus, TsError> {
        parse_status(&self.run(&["status".into(), "--json".into()])?)
    }
    fn serve(&self, https_port: u16, target_port: u16) -> Result<(), TsError> {
        self.run(&serve_args(https_port, target_port)).map(|_| ())
    }
    fn unserve(&self, https_port: u16) -> Result<(), TsError> {
        self.run(&unserve_args(https_port)).map(|_| ())
    }
}

/// A TailscaleApi that runs nothing and reports the CLI as missing. It is the default until production wiring
/// installs the real CLI, so code that forgets to inject one can never reach the machine's Tailscale.
pub struct Unavailable;

impl TailscaleApi for Unavailable {
    fn status(&self) -> Result<TsStatus, TsError> {
        Err(TsError::NotInstalled)
    }
    fn serve(&self, _https_port: u16, _target_port: u16) -> Result<(), TsError> {
        Err(TsError::NotInstalled)
    }
    fn unserve(&self, _https_port: u16) -> Result<(), TsError> {
        Err(TsError::NotInstalled)
    }
}

pub mod testing {
    use super::*;
    use std::sync::Mutex;

    /// An in-memory TailscaleApi that records `serve` calls as command lines and counts `status` calls, for other
    /// crates' tests.
    pub struct RecordingTailscale {
        status: Mutex<Result<TsStatus, TsError>>,
        calls: Mutex<Vec<String>>,
        status_calls: Mutex<usize>,
    }

    impl RecordingTailscale {
        pub fn new(status: Result<TsStatus, TsError>) -> Self {
            Self { status: Mutex::new(status), calls: Mutex::new(Vec::new()), status_calls: Mutex::new(0) }
        }
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        /// How many times `status` was asked.
        pub fn status_calls(&self) -> usize {
            *self.status_calls.lock().unwrap()
        }
        pub fn set_status(&self, status: Result<TsStatus, TsError>) {
            *self.status.lock().unwrap() = status;
        }
    }

    impl TailscaleApi for RecordingTailscale {
        fn status(&self) -> Result<TsStatus, TsError> {
            *self.status_calls.lock().unwrap() += 1;
            self.status.lock().unwrap().clone()
        }
        fn serve(&self, https_port: u16, target_port: u16) -> Result<(), TsError> {
            self.calls.lock().unwrap().push(serve_args(https_port, target_port).join(" "));
            Ok(())
        }
        fn unserve(&self, https_port: u16) -> Result<(), TsError> {
            self.calls.lock().unwrap().push(unserve_args(https_port).join(" "));
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"{
      "BackendState": "Running",
      "Self": { "DNSName": "box.tail1234.ts.net.", "UserID": 42, "HostName": "box" },
      "User": { "42": { "ID": 42, "LoginName": "me@example.com", "DisplayName": "Me" } },
      "CertDomains": ["box.tail1234.ts.net"],
      "MagicDNSSuffix": "tail1234.ts.net"
    }"#;

    #[test]
    fn parses_running_status() {
        let s = parse_status(STATUS).unwrap();
        assert_eq!(s.dns_name, "box.tail1234.ts.net");
        assert_eq!(s.self_login.as_deref(), Some("me@example.com"));
        assert!(s.https_enabled);
    }

    #[test]
    fn https_disabled_when_cert_domains_empty_or_null() {
        let empty = STATUS.replace(r#"["box.tail1234.ts.net"]"#, "[]");
        assert!(!parse_status(&empty).unwrap().https_enabled);
        let null = STATUS.replace(r#"["box.tail1234.ts.net"]"#, "null");
        assert!(!parse_status(&null).unwrap().https_enabled);
    }

    #[test]
    fn not_running_and_garbage() {
        let stopped = STATUS.replace(r#""Running""#, r#""NeedsLogin""#);
        assert!(matches!(parse_status(&stopped), Err(TsError::NotRunning(s)) if s.contains("NeedsLogin")));
        assert!(matches!(parse_status("nope"), Err(TsError::BadStatus(_))));
        let no_dns = STATUS.replace(r#""box.tail1234.ts.net.""#, r#""""#);
        assert!(matches!(parse_status(&no_dns), Err(TsError::BadStatus(_))));
    }

    #[test]
    fn missing_user_entry_is_no_login() {
        let other = STATUS.replace(r#""42": {"#, r#""7": {"#);
        assert_eq!(parse_status(&other).unwrap().self_login, None);
    }

    #[test]
    fn command_lines() {
        assert_eq!(serve_args(443, 7420), ["serve", "--bg", "--https=443", "http://127.0.0.1:7420"]);
        assert_eq!(unserve_args(8401), ["serve", "--https=8401", "off"]);
    }

    #[test]
    fn classifies_cli_failures() {
        assert!(matches!(
            classify_failure("failed to connect to local Tailscale daemon for /localapi/v0/status; not running?"),
            TsError::NotRunning(_)
        ));
        assert!(matches!(classify_failure("Tailscale is stopped."), TsError::NotRunning(_)));
        assert!(matches!(classify_failure("something else"), TsError::Command(s) if s == "something else"));
    }

    #[test]
    fn recording_fake_records_calls() {
        let f = testing::RecordingTailscale::new(Ok(parse_status(STATUS).unwrap()));
        f.serve(443, 7421).unwrap();
        f.unserve(443).unwrap();
        assert_eq!(f.calls(), ["serve --bg --https=443 http://127.0.0.1:7421", "serve --https=443 off"]);
        assert_eq!(f.status_calls(), 0);
        f.set_status(Err(TsError::NotInstalled));
        assert!(matches!(f.status(), Err(TsError::NotInstalled)));
        assert_eq!(f.status_calls(), 1);
    }

    #[test]
    fn unavailable_never_runs_anything() {
        assert!(matches!(Unavailable.status(), Err(TsError::NotInstalled)));
        assert!(matches!(Unavailable.serve(443, 1), Err(TsError::NotInstalled)));
        assert!(matches!(Unavailable.unserve(443), Err(TsError::NotInstalled)));
    }
}
