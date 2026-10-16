//! Updates from GitHub releases through the `gh` CLI (the release repository is internal, so the user's own gh login
//! is what grants access). Checking and asset selection are pure; installing hands off to a detached helper script
//! that runs after this app exits.

use crate::platform::Os;
use pdt_core::brand::{DAEMON_BIN, PRODUCT_NAME, RELEASE_REPO};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// Parses `1.2.3` or `v1.2.3`; a pre-release or build suffix (`-rc.1`, `+abc`) is ignored.
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = Version(parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: Version,
    pub assets: Vec<String>,
}

/// Parses `gh release view --json tagName,assets`.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("gh printed something unexpected: {e}"))?;
    let tag = v["tagName"].as_str().ok_or("the release has no tag")?.to_string();
    let version = parse_version(&tag).ok_or_else(|| format!("the release tag {tag:?} is not a version (vX.Y.Z)"))?;
    let assets = v["assets"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(Release { tag, version, assets })
}

/// The `gh release download --pattern` for this platform's release asset.
pub fn asset_pattern(os: Os) -> Option<&'static str> {
    match os {
        Os::Windows => Some("*-setup.exe"),
        Os::Mac => Some("*.app.tar.gz"),
        Os::Other => None,
    }
}

/// The release asset for `os`, if the release has one.
pub fn select_asset(assets: &[String], os: Os) -> Option<&str> {
    let suffix = asset_pattern(os)?.trim_start_matches('*');
    assets.iter().map(String::as_str).find(|a| a.len() > suffix.len() && a.ends_with(suffix))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Fits in the menu label: "Update check failed (gh <short>)".
    pub short: String,
    /// Shown in the dialog: what happened and how to fix it.
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Updates don't apply to this app (the dev profile, or not the installed stable app); the text says why.
    Disabled(String),
    Checking,
    UpToDate,
    Available(Release),
    Failed(Failure),
}

/// Explains a failed `gh` run. `not_found` means gh itself could not be started.
pub fn classify_failure(not_found: bool, stderr: &str) -> Failure {
    let fail = |short: &str, detail: String| Failure { short: short.into(), detail };
    let err = stderr.trim();
    let lower = err.to_ascii_lowercase();
    if not_found {
        return fail(
            "not installed",
            "The GitHub CLI (gh) was not found. Install it from https://cli.github.com, then run `gh auth login` in \
             a terminal."
                .into(),
        );
    }
    if lower.contains("gh auth login") || lower.contains("http 401") || lower.contains("bad credentials") {
        return fail(
            "not logged in",
            format!("gh is not logged in to GitHub.\n\nFix: run `gh auth login` in a terminal.\n\ngh said: {err}"),
        );
    }
    if lower.contains("release not found") || lower.contains("http 404") {
        return fail(
            "no release",
            format!(
                "No release was found in {RELEASE_REPO}. Either none has been published yet, or the GitHub account \
                 gh is logged in as cannot see the repository.\n\nFix: run `gh auth login` with an account that has \
                 access to {RELEASE_REPO}.\n\ngh said: {err}"
            ),
        );
    }
    let network = ["dial tcp", "connect", "timeout", "no such host", "could not resolve", "tls"];
    if network.iter().any(|n| lower.contains(n)) {
        return fail(
            "network error",
            format!("gh could not reach GitHub. Check the network connection and try again.\n\ngh said: {err}"),
        );
    }
    fail(
        "error",
        format!(
            "gh failed: {}\n\nIf it is an authentication problem, run `gh auth login` in a terminal.",
            if err.is_empty() { "no error message" } else { err }
        ),
    )
}

/// Turns gh's release JSON into a status for the app running `current`.
pub fn evaluate(current: &str, json: &str, os: Os) -> Status {
    let release = match parse_release(json) {
        Ok(r) => r,
        Err(e) => return Status::Failed(classify_failure(false, &e)),
    };
    let Some(current) = parse_version(current) else {
        return Status::Failed(Failure {
            short: "error".into(),
            detail: format!("this app's version {current:?} is not X.Y.Z"),
        });
    };
    if release.version <= current {
        return Status::UpToDate;
    }
    if select_asset(&release.assets, os).is_none() {
        return Status::Failed(Failure {
            short: "no installer".into(),
            detail: format!(
                "Release {} has no asset matching {} for this platform.",
                release.tag,
                asset_pattern(os).unwrap_or("(unsupported platform)")
            ),
        });
    }
    Status::Available(release)
}

/// The update menu item's text and whether it is enabled.
pub fn menu_label(status: &Status) -> (String, bool) {
    match status {
        Status::Disabled(why) => (why.clone(), false),
        Status::Checking => ("Checking for updates…".into(), false),
        Status::UpToDate => ("Check for updates".into(), true),
        Status::Available(r) => (format!("Update to v{}…", r.version), true),
        Status::Failed(f) => (format!("Update check failed (gh {})", f.short), true),
    }
}

/// Updates replace the installed stable app, so they are only offered when this exe *is* that app: on Windows the
/// per-user NSIS install dir `<local app data>\<product>`, on macOS an `.app` bundle. Err gives the menu text.
pub fn install_check(os: Os, exe: &Path, local_app_data: Option<&Path>) -> Result<(), String> {
    let installed = match os {
        // Compared as text, not with Path, so the check means the same on every host (tests run on macOS too,
        // where `\` is not a separator).
        Os::Windows => match local_app_data {
            Some(lad) => {
                let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
                let exe = norm(exe);
                let dir = exe.rsplit_once('\\').map_or("", |(d, _)| d);
                let expected = format!("{}\\{}", norm(lad).trim_end_matches('\\'), PRODUCT_NAME.to_lowercase());
                dir == expected
            }
            None => false,
        },
        Os::Mac => app_bundle_of(exe).is_some(),
        Os::Other => false,
    };
    if installed { Ok(()) } else { Err("Updates apply to the installed app only".into()) }
}

/// Where to find gh. GUI apps on macOS get a minimal PATH, so Homebrew's locations are tried explicitly.
pub fn gh_candidates(os: Os) -> Vec<PathBuf> {
    match os {
        Os::Mac => vec![PathBuf::from("/opt/homebrew/bin/gh"), PathBuf::from("/usr/local/bin/gh")],
        _ => Vec::new(),
    }
}

fn gh() -> Command {
    let program = gh_candidates(Os::current()).into_iter().find(|p| p.is_file()).unwrap_or_else(|| "gh".into());
    let mut cmd = Command::new(program);
    cmd.env("GH_PROMPT_DISABLED", "1").env("GH_NO_UPDATE_NOTIFIER", "1").stdin(Stdio::null());
    no_window(&mut cmd);
    cmd
}

fn no_window(_cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        _cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

fn run_gh(args: &[&str]) -> Result<String, Failure> {
    match gh().args(args).output() {
        Err(e) => Err(classify_failure(e.kind() == std::io::ErrorKind::NotFound, &e.to_string())),
        Ok(o) if !o.status.success() => Err(classify_failure(false, &String::from_utf8_lossy(&o.stderr))),
        Ok(o) => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
    }
}

/// Asks GitHub (through gh) for the latest release. Blocks; call it off the main thread.
pub fn check(current: &str) -> Status {
    match run_gh(&["release", "view", "--repo", RELEASE_REPO, "--json", "tagName,assets"]) {
        Ok(json) => evaluate(current, &json, Os::current()),
        Err(f) => Status::Failed(f),
    }
}

/// Downloads this platform's asset of `release` into `dir` and returns its path.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf, String> {
    let os = Os::current();
    let pattern = asset_pattern(os).ok_or("updates are not supported on this platform")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let dir_s = dir.to_string_lossy();
    run_gh(&[
        "release",
        "download",
        &release.tag,
        "--repo",
        RELEASE_REPO,
        "--pattern",
        pattern,
        "--dir",
        &dir_s,
        "--clobber",
    ])
    .map_err(|f| f.detail)?;
    let names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let name = select_asset(&names, os).ok_or_else(|| format!("gh downloaded no {pattern} into {}", dir.display()))?;
    Ok(dir.join(name))
}

/// Quotes for a PowerShell single-quoted string.
#[cfg_attr(not(windows), allow(dead_code))]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Quotes for a POSIX shell single-quoted string.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The Windows helper: waits for the tray (`pid`) to exit, stops our pdtd on `port`, runs the NSIS installer
/// silently, deletes `download_dir` after a successful install, then starts the installed app from `install_dir`
/// without any `PDT_*` variables.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn windows_script(
    pid: u32,
    port: u16,
    setup: &Path,
    download_dir: &Path,
    install_dir: &Path,
    log: &Path,
) -> String {
    let product = ps_quote(PRODUCT_NAME);
    let daemon = ps_quote(DAEMON_BIN);
    let setup = ps_quote(&setup.to_string_lossy());
    let install_dir = ps_quote(&install_dir.to_string_lossy());
    let download_dir = ps_quote(&download_dir.to_string_lossy());
    let log = ps_quote(&log.to_string_lossy());
    format!(
        r#"# Generated by {PRODUCT_NAME}: installs an update after the tray app exits.
$ErrorActionPreference = 'Stop'
$log = {log}
$port = {port}
$installDir = {install_dir}
function Log([string]$m) {{ try {{ Add-Content -LiteralPath $log -Value ('{{0}} {{1}}' -f (Get-Date -Format o), $m) }} catch {{}} }}
function Test-Port([int]$p) {{
    try {{ $c = New-Object Net.Sockets.TcpClient; $c.Connect('127.0.0.1', $p); $c.Close(); $true }} catch {{ $false }}
}}
function In-InstallDir($proc) {{ $proc.Path -and $proc.Path.StartsWith($installDir + '\', [StringComparison]::OrdinalIgnoreCase) }}
$installed = $false
try {{
    Log 'Waiting for the tray app (pid {pid}) to exit'
    $tray = Get-Process -Id {pid} -ErrorAction SilentlyContinue
    if ($tray) {{ [void]$tray.WaitForExit(30000) }}
    if (Test-Port $port) {{
        $health = $null
        try {{ $health = Invoke-RestMethod -Uri "http://127.0.0.1:$port/api/health" -TimeoutSec 5 }} catch {{}}
        if ($health -and $health.name -eq {product}) {{
            Log "Stopping {DAEMON_BIN} on port $port"
            Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/api/shutdown" -TimeoutSec 10 | Out-Null
        }}
        $deadline = (Get-Date).AddSeconds(20)
        while ((Test-Port $port) -and ((Get-Date) -lt $deadline)) {{ Start-Sleep -Milliseconds 250 }}
        if (Test-Port $port) {{ throw "port $port is still in use after 20 seconds" }}
    }}
    Get-Process -Name {daemon} -ErrorAction SilentlyContinue | Where-Object {{ In-InstallDir $_ }} |
        ForEach-Object {{ [void]$_.WaitForExit(10000) }}
    Log 'Running the installer'
    $p = Start-Process -FilePath {setup} -ArgumentList '/S' -Wait -PassThru
    if ($p.ExitCode -ne 0) {{ throw "the installer failed (exit $($p.ExitCode))" }}
    Log 'Installed'
    $installed = $true
}} catch {{
    Log "Update failed: $_"
}}
if ($installed) {{
    try {{ Remove-Item -LiteralPath {download_dir} -Recurse -Force }} catch {{ Log "Could not delete the download: $_" }}
}}
try {{
    $app = Get-ChildItem -LiteralPath $installDir -Filter '*.exe' |
        Where-Object {{ $_.BaseName -notin @({daemon}, 'uninstall') }} | Select-Object -First 1
    if (-not $app) {{ throw "no app exe was found in $installDir" }}
    # Start with no PDT_* variables so the stable app never inherits a dev setup.
    $psi = New-Object Diagnostics.ProcessStartInfo $app.FullName
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $installDir
    foreach ($k in @($psi.EnvironmentVariables.Keys)) {{ if ($k -like 'PDT_*') {{ $psi.EnvironmentVariables.Remove($k) }} }}
    [Diagnostics.Process]::Start($psi) | Out-Null
    Log "Started $($app.Name)"
}} catch {{
    Log "Could not start the app: $_"
}}
"#
    )
}

/// The macOS helper: waits for the tray (`pid`) to exit, stops our pdtd on `port` and waits for the bundle's pdtd
/// process to exit, replaces `bundle` with the `.app` from `tarball`, deletes `download_dir` after a successful
/// install, then opens the bundle without any `PDT_*` variables.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn macos_script(pid: u32, port: u16, tarball: &Path, download_dir: &Path, bundle: &Path, log: &Path) -> String {
    let tarball = sh_quote(&tarball.to_string_lossy());
    let bundle = sh_quote(&bundle.to_string_lossy());
    let log = sh_quote(&log.to_string_lossy());
    let download_dir = sh_quote(&download_dir.to_string_lossy());
    let name_match = sh_quote(&format!("\"name\":\"{PRODUCT_NAME}\""));
    format!(
        r#"#!/bin/sh
# Generated by {PRODUCT_NAME}: installs an update after the tray app exits.
exec >>{log} 2>&1
port={port}
bundle={bundle}
up() {{ curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$port/api/health"; }}
echo "$(date) waiting for the tray app (pid {pid}) to exit"
i=0; while kill -0 {pid} 2>/dev/null && [ $i -lt 150 ]; do sleep 0.2; i=$((i+1)); done
if curl -s --max-time 5 "http://127.0.0.1:$port/api/health" | grep -qF {name_match}; then
  echo "stopping {DAEMON_BIN} on port $port"
  curl -s -X POST --max-time 10 "http://127.0.0.1:$port/api/shutdown" >/dev/null
fi
i=0; while up && [ $i -lt 80 ]; do sleep 0.25; i=$((i+1)); done
# pdtd frees the port before it finishes exiting (it drains connections and closes its store first).
i=0; while pgrep -f "$bundle/Contents/MacOS/{DAEMON_BIN}" >/dev/null 2>&1 && [ $i -lt 80 ]; do sleep 0.25; i=$((i+1)); done
stage="$(mktemp -d "$(dirname "$bundle")/.update.XXXXXX" 2>/dev/null || mktemp -d)"
if up; then
  echo "port $port is still in use after 20 seconds; not updating"
elif tar -xzf {tarball} -C "$stage"; then
  new="$(find "$stage" -maxdepth 1 -name '*.app' -type d | head -n 1)"
  if [ -z "$new" ]; then
    echo "the archive has no .app bundle"
  else
    xattr -dr com.apple.quarantine "$new" 2>/dev/null || true
    rm -rf "$bundle.old"
    if mv "$bundle" "$bundle.old"; then
      if mv "$new" "$bundle"; then
        rm -rf "$bundle.old"
        rm -rf {download_dir}
        echo "installed"
      else
        mv "$bundle.old" "$bundle"
        echo "could not move the new bundle into place"
      fi
    fi
  fi
else
  echo "could not extract the update"
fi
rm -rf "$stage"
for v in $(env | sed -n 's/^\(PDT_[A-Za-z0-9_]*\)=.*/\1/p'); do unset "$v"; done
open "$bundle"
"#
    )
}

/// The `.app` bundle that contains `exe` (`…/X.app/Contents/MacOS/x` → `…/X.app`).
pub fn app_bundle_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// Writes the helper for `asset` into `dir` and starts it detached. The caller exits right after.
pub fn launch_helper(asset: &Path, dir: &Path, port: u16, log: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let pid = std::process::id();
    let mut cmd;
    #[cfg(windows)]
    {
        let install_dir = exe.parent().ok_or("the app has no install folder")?;
        let script = dir.join("update.ps1");
        // A BOM makes Windows PowerShell 5.1 read the script as UTF-8 (paths can be non-ASCII).
        let text = format!("\u{feff}{}", windows_script(pid, port, asset, dir, install_dir, log).replace('\n', "\r\n"));
        std::fs::write(&script, text).map_err(|e| format!("could not write {}: {e}", script.display()))?;
        let ps = std::env::var_os("SystemRoot")
            .map(|r| PathBuf::from(r).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| "powershell.exe".into());
        cmd = Command::new(ps);
        cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
            .arg(&script);
    }
    #[cfg(not(windows))]
    {
        let bundle = app_bundle_of(&exe).ok_or("the app is not running from an .app bundle")?;
        let script = dir.join("update.sh");
        std::fs::write(&script, macos_script(pid, port, asset, dir, &bundle, log))
            .map_err(|e| format!("could not write {}: {e}", script.display()))?;
        cmd = Command::new("/bin/sh");
        cmd.arg(&script);
    }
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    spawn_detached(cmd).map_err(|e| format!("could not start the update helper: {e}"))
}

#[cfg(windows)]
fn spawn_detached(mut cmd: Command) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    // Break away from a job the app may run in, so the helper outlives it; not every job allows that.
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
    if cmd.spawn().is_ok() {
        return Ok(());
    }
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    cmd.spawn().map(|_| ())
}

#[cfg(not(windows))]
fn spawn_detached(mut cmd: Command) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    cmd.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v0.1.42"), Some(Version(0, 1, 42)));
        assert_eq!(parse_version("0.1.0"), Some(Version(0, 1, 0)));
        assert_eq!(parse_version("v1.2.3-rc.1"), Some(Version(1, 2, 3)));
        assert_eq!(parse_version("1.2.3+build"), Some(Version(1, 2, 3)));
        for bad in ["", "v", "1.2", "1.2.3.4", "v1.x.3", "latest"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(Version(0, 1, 10) > Version(0, 1, 9));
        assert!(Version(0, 2, 0) > Version(0, 1, 99));
        assert_eq!(Version(0, 1, 7).to_string(), "0.1.7");
    }

    #[test]
    fn parses_gh_release_json() {
        let json = r#"{"assets":[{"name":"Personal Dev Team_0.1.5_x64-setup.exe","url":"u"},
            {"name":"Personal Dev Team.app.tar.gz"}],"tagName":"v0.1.5"}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.tag, "v0.1.5");
        assert_eq!(r.version, Version(0, 1, 5));
        assert_eq!(r.assets.len(), 2);
        assert!(parse_release("not json").is_err());
        assert!(parse_release(r#"{"tagName":"nightly","assets":[]}"#).unwrap_err().contains("nightly"));
        assert!(parse_release(r#"{"assets":[]}"#).is_err());
    }

    #[test]
    fn selects_the_platform_asset() {
        let assets = names(&["notes.txt", "Personal Dev Team_0.1.5_x64-setup.exe", "Personal Dev Team.app.tar.gz"]);
        assert_eq!(select_asset(&assets, Os::Windows), Some("Personal Dev Team_0.1.5_x64-setup.exe"));
        assert_eq!(select_asset(&assets, Os::Mac), Some("Personal Dev Team.app.tar.gz"));
        assert_eq!(select_asset(&assets, Os::Other), None);
        assert_eq!(select_asset(&names(&["-setup.exe", "x.tar.gz"]), Os::Windows), None);
        assert_eq!(select_asset(&names(&["x.tar.gz"]), Os::Mac), None);
    }

    #[test]
    fn evaluates_against_the_running_version() {
        let json = r#"{"tagName":"v0.1.5","assets":[{"name":"a_0.1.5_x64-setup.exe"},{"name":"A.app.tar.gz"}]}"#;
        assert!(matches!(evaluate("0.1.4", json, Os::Windows), Status::Available(r) if r.tag == "v0.1.5"));
        assert_eq!(evaluate("0.1.5", json, Os::Windows), Status::UpToDate);
        assert_eq!(evaluate("0.1.6", json, Os::Mac), Status::UpToDate);
        let no_mac = r#"{"tagName":"v0.1.5","assets":[{"name":"a-setup.exe"}]}"#;
        assert!(matches!(evaluate("0.1.0", no_mac, Os::Mac), Status::Failed(f) if f.short == "no installer"));
        assert!(matches!(evaluate("0.1.0", "{", Os::Windows), Status::Failed(_)));
    }

    #[test]
    fn classifies_gh_failures() {
        assert_eq!(classify_failure(true, "program not found").short, "not installed");
        let not_logged_in =
            "To get started with GitHub CLI, please run:  gh auth login\nAlternatively, populate GH_TOKEN";
        let f = classify_failure(false, not_logged_in);
        assert_eq!(f.short, "not logged in");
        assert!(f.detail.contains("gh auth login"));
        let bad_token =
            "HTTP 401: Bad credentials (https://api.github.com/...)\nTry authenticating with:  gh auth login";
        assert_eq!(classify_failure(false, bad_token).short, "not logged in");
        let f = classify_failure(false, "release not found");
        assert_eq!(f.short, "no release");
        assert!(f.detail.contains(RELEASE_REPO) && f.detail.contains("gh auth login"));
        let net = "Get \"https://api.github.com/...\": proxyconnect tcp: dial tcp 127.0.0.1:1: connectex: refused";
        assert_eq!(classify_failure(false, net).short, "network error");
        let f = classify_failure(false, "something odd");
        assert_eq!(f.short, "error");
        assert!(f.detail.contains("something odd"));
    }

    #[test]
    fn menu_labels_reflect_the_last_check() {
        let r = Release { tag: "v0.1.9".into(), version: Version(0, 1, 9), assets: vec![] };
        assert_eq!(menu_label(&Status::Available(r)), ("Update to v0.1.9…".into(), true));
        assert_eq!(menu_label(&Status::UpToDate), ("Check for updates".into(), true));
        assert!(!menu_label(&Status::Checking).1);
        let why = "Updates apply to the stable app".to_string();
        assert_eq!(menu_label(&Status::Disabled(why.clone())), (why, false));
        let f = Failure { short: "not logged in".into(), detail: String::new() };
        assert_eq!(menu_label(&Status::Failed(f)), ("Update check failed (gh not logged in)".into(), true));
    }

    #[test]
    fn windows_script_waits_stops_installs_and_restarts() {
        let s = windows_script(
            4242,
            7420,
            Path::new(r"C:\Temp\pdt-update\v0.1.5\PDT_0.1.5_x64-setup.exe"),
            Path::new(r"C:\Temp\pdt-update\v0.1.5"),
            Path::new(r"C:\Users\o'neil\AppData\Local\Personal Dev Team"),
            Path::new(r"C:\data\logs\update.log"),
        );
        assert!(s.contains("Get-Process -Id 4242"));
        assert!(s.contains("$port = 7420"));
        assert!(s.contains("/api/shutdown"));
        assert!(s.contains("AddSeconds(20)"));
        assert!(s.contains(r"-FilePath 'C:\Temp\pdt-update\v0.1.5\PDT_0.1.5_x64-setup.exe' -ArgumentList '/S' -Wait"));
        assert!(s.contains(r"$installDir = 'C:\Users\o''neil\AppData\Local\Personal Dev Team'"));
        assert!(s.contains(r"$log = 'C:\data\logs\update.log'"));
        assert!(s.contains("$health.name -eq 'Personal Dev Team'"));
        assert!(s.contains("@('pdtd', 'uninstall')"));
        assert!(s.contains("-like 'PDT_*'"));
        // The installer runs, then the download is deleted (only after a successful install), then the app restarts.
        let install = s.find("-ArgumentList '/S'").unwrap();
        let cleanup = s.find(r"Remove-Item -LiteralPath 'C:\Temp\pdt-update\v0.1.5' -Recurse").unwrap();
        assert!(install < cleanup && cleanup < s.find("[Diagnostics.Process]::Start").unwrap());
        assert!(s.contains("if ($installed) {"));
        // Finding and starting the app is inside a try, so its failures are logged.
        assert!(s.contains("Log \"Could not start the app: $_\""));
    }

    #[test]
    fn macos_script_waits_stops_replaces_and_opens() {
        let s = macos_script(
            99,
            7420,
            Path::new("/tmp/pdt-update/v0.1.5/Personal Dev Team.app.tar.gz"),
            Path::new("/tmp/pdt-update/v0.1.5"),
            Path::new("/Users/me/Applications/Personal Dev Team.app"),
            Path::new("/Users/me/Library/Application Support/pdt/logs/update.log"),
        );
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("kill -0 99"));
        assert!(s.contains("port=7420"));
        assert!(s.contains("/api/shutdown"));
        assert!(s.contains("tar -xzf '/tmp/pdt-update/v0.1.5/Personal Dev Team.app.tar.gz'"));
        assert!(s.contains("bundle='/Users/me/Applications/Personal Dev Team.app'"));
        assert!(s.contains("exec >>'/Users/me/Library/Application Support/pdt/logs/update.log'"));
        assert!(s.contains(r#"grep -qF '"name":"Personal Dev Team"'"#));
        assert!(s.trim_end().ends_with("open \"$bundle\""));
        // Waits for the old pdtd process (not just the port) before replacing the bundle.
        let wait = s.find("pgrep -f \"$bundle/Contents/MacOS/pdtd\"").unwrap();
        assert!(wait < s.find("tar -xzf").unwrap());
        let cleanup = s.find("rm -rf '/tmp/pdt-update/v0.1.5'").unwrap();
        assert!(s.find("echo \"installed\"").unwrap() > cleanup);
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn updates_only_the_installed_stable_app() {
        let lad = Path::new(r"C:\Users\me\AppData\Local");
        let installed = lad.join("Personal Dev Team").join("pdt-desktop.exe");
        assert_eq!(install_check(Os::Windows, &installed, Some(lad)), Ok(()));
        let upper = Path::new(r"C:\Users\me\AppData\Local\PERSONAL DEV TEAM\pdt-desktop.exe");
        assert_eq!(install_check(Os::Windows, upper, Some(lad)), Ok(()));
        let dev = Path::new(r"C:\work\ai-dev-shop\target\debug\pdt-desktop.exe");
        assert!(install_check(Os::Windows, dev, Some(lad)).unwrap_err().contains("installed app only"));
        assert!(install_check(Os::Windows, &installed, None).is_err());
        let bundle = Path::new("/Applications/Personal Dev Team.app/Contents/MacOS/pdt-desktop");
        assert_eq!(install_check(Os::Mac, bundle, None), Ok(()));
        assert!(install_check(Os::Mac, Path::new("/Users/me/target/debug/pdt-desktop"), None).is_err());
    }

    #[test]
    fn finds_the_app_bundle() {
        let exe = Path::new("/Users/me/Applications/Personal Dev Team.app/Contents/MacOS/pdt-desktop");
        assert_eq!(app_bundle_of(exe), Some(PathBuf::from("/Users/me/Applications/Personal Dev Team.app")));
        assert_eq!(app_bundle_of(Path::new("/usr/local/bin/pdt-desktop")), None);
    }

    #[test]
    fn gh_is_looked_up_in_homebrew_on_macos() {
        assert!(gh_candidates(Os::Mac).contains(&PathBuf::from("/opt/homebrew/bin/gh")));
        assert!(gh_candidates(Os::Windows).is_empty());
    }
}
