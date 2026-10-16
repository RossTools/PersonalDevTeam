//! Helpers shared by the pdtd integration tests (`mod common;`). They start the real pdtd binary on a free port with
//! a temp data dir, and never touch the stable instance.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Daemon(pub Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

pub fn start(port: u16, data: &Path) -> Daemon {
    start_with(port, data, &[])
}

/// Starts pdtd with every inherited `PDT_*` variable removed, plus `extra_env`.
pub fn start_with(port: u16, data: &Path, extra_env: &[(&str, &str)]) -> Daemon {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pdtd"));
    for (k, _) in std::env::vars() {
        if k.starts_with("PDT_") {
            cmd.env_remove(k);
        }
    }
    let child = cmd
        .env("PDT_PROFILE", "dev")
        .env("PDT_PORT", port.to_string())
        .env("PDT_DATA_DIR", data)
        .env("PDT_SECRET_STORE", "memory")
        // Never the machine's Tailscale (it serves the stable instance): any call would find no CLI.
        .env("PDT_TAILSCALE_BIN", data.join("no-such-tailscale"))
        .envs(extra_env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let d = Daemon(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if http(port, "GET", "/api/health", None).0 == 200 {
            return d;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("pdtd did not become healthy");
}

pub fn http(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, serde_json::Value) {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return (0, serde_json::Value::Null) };
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    if s.write_all(req.as_bytes()).is_err() {
        return (0, serde_json::Value::Null);
    }
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let Some((head, body)) = text.split_once("\r\n\r\n") else { return (0, serde_json::Value::Null) };
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, serde_json::from_str(body).unwrap_or(serde_json::Value::Null))
}

pub fn wait_exit(d: &mut Daemon, within: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + within;
    loop {
        if let Some(status) = d.0.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "pdtd did not exit");
        std::thread::sleep(Duration::from_millis(100));
    }
}
