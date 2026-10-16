use pdt_tailscale::{TailscaleApi, TailscaleCli, TsError};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fake(tmp: &std::path::Path) -> (TailscaleCli, PathBuf) {
    let status = tmp.join("status.json");
    std::fs::write(
        &status,
        r#"{"BackendState":"Running","Self":{"DNSName":"box.t.ts.net.","UserID":1},
            "User":{"1":{"LoginName":"me@example.com"}},"CertDomains":["box.t.ts.net"]}"#,
    )
    .unwrap();
    let log = tmp.join("calls.log");
    let cli = TailscaleCli::new(PathBuf::from(env!("CARGO_BIN_EXE_fake-tailscale")))
        .with_env("FAKE_TAILSCALE_STATUS", &status.to_string_lossy())
        .with_env("FAKE_TAILSCALE_LOG", &log.to_string_lossy());
    (cli, log)
}

#[test]
fn status_serve_and_unserve_go_through_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let (cli, log) = fake(tmp.path());
    let s = cli.status().unwrap();
    assert_eq!((s.dns_name.as_str(), s.self_login.as_deref()), ("box.t.ts.net", Some("me@example.com")));
    cli.serve(443, 7421).unwrap();
    cli.unserve(443).unwrap();
    let calls = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        ["status --json", "serve --bg --https=443 http://127.0.0.1:7421", "serve --https=443 off"]
    );
}

#[test]
fn a_stopped_daemon_and_a_missing_cli_are_typed() {
    let tmp = tempfile::tempdir().unwrap();
    let (cli, _) = fake(tmp.path());
    let down = cli.with_env("FAKE_TAILSCALE_DOWN", "1");
    assert!(matches!(down.status(), Err(TsError::NotRunning(_))));
    let missing = TailscaleCli::new(tmp.path().join("no-such-tailscale"));
    assert!(matches!(missing.status(), Err(TsError::NotInstalled)));
}

#[test]
fn a_hung_cli_is_killed_after_the_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let (cli, _) = fake(tmp.path());
    let slow = cli.with_env("FAKE_TAILSCALE_SLEEP_MS", "10000").with_timeout(Duration::from_millis(300));
    let started = Instant::now();
    assert!(matches!(slow.status(), Err(TsError::TimedOut)));
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
}
