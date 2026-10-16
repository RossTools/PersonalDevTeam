//! Tiny loopback JSON client for the tray (no extra dependencies).

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub fn call(port: u16, method: &str, path: &str, body: Option<&str>) -> Result<(u16, serde_json::Value), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(900)));
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("not an HTTP response")?;
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("no status")?;
    Ok((status, serde_json::from_str(body).unwrap_or(serde_json::Value::Null)))
}

/// The Update menu item for a `/api/host/update` status: (label, enabled).
pub fn update_label(status: &serde_json::Value) -> (String, bool) {
    match status["state"].as_str() {
        Some("available") => (format!("Update to v{}…", status["latest"]["version"].as_str().unwrap_or("?")), true),
        Some("checking") => ("Checking for updates…".into(), false),
        Some("downloading") => ("Downloading the update…".into(), false),
        Some("installing") => ("Installing the update…".into(), false),
        Some("disabled") => ("Updates are off for this app".into(), false),
        _ if status["error"].is_string() => ("Update check failed — see Settings".into(), true),
        _ => ("Check for updates".into(), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels() {
        assert_eq!(
            update_label(&json!({"state": "available", "latest": {"version": "0.1.9"}})),
            ("Update to v0.1.9…".into(), true)
        );
        assert!(!update_label(&json!({"state": "installing"})).1);
        assert!(!update_label(&json!({"state": "disabled"})).1);
        assert!(update_label(&json!({"state": "idle", "error": "x"})).1);
        assert_eq!(update_label(&json!({"state": "idle", "error": null})), ("Check for updates".into(), true));
    }

    #[test]
    fn calls_send_the_body_and_read_status_and_json() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !String::from_utf8_lossy(&req).contains("{\"force\":true}") {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0, "request ended early: {}", String::from_utf8_lossy(&req));
                req.extend_from_slice(&buf[..n]);
            }
            s.write_all(b"HTTP/1.1 409 Conflict\r\ncontent-type: application/json\r\n\r\n{\"code\":\"host_busy\"}")
                .unwrap();
            String::from_utf8(req).unwrap()
        });
        let (status, v) = call(port, "POST", "/api/host/update/install", Some("{\"force\":true}")).unwrap();
        assert_eq!((status, v["code"].as_str()), (409, Some("host_busy")));
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /api/host/update/install HTTP/1.0\r\n"), "{req}");
        assert!(req.contains("Content-Length: 14\r\n"), "{req}");
    }
}
