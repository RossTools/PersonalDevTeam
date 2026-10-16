use pdt_core::brand::PRODUCT_NAME;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    Free,
    Ours { version: String },
    Foreign(String),
}

pub fn probe(port: u16) -> Probe {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else { return Probe::Free };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    if let Err(e) = s.write_all(format!("GET /api/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()) {
        return Probe::Foreign(e.to_string());
    }
    let mut buf = Vec::new();
    if let Err(e) = s.take(64 * 1024).read_to_end(&mut buf) {
        return Probe::Foreign(e.to_string());
    }
    classify_health(&buf)
}

pub fn classify_health(raw: &[u8]) -> Probe {
    let text = String::from_utf8_lossy(raw);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Probe::Foreign("it did not answer like a web server".into());
    };
    let status = head.lines().next().unwrap_or("");
    if !status.starts_with("HTTP/1.") || status.split_whitespace().nth(1) != Some("200") {
        return Probe::Foreign(format!("its health check answered {status:?}"));
    }
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(v) if v["name"] == PRODUCT_NAME => Probe::Ours { version: v["version"].as_str().unwrap_or("").to_string() },
        Ok(v) => Probe::Foreign(format!("it identifies as {}", v["name"])),
        Err(_) => Probe::Foreign("its health check is not JSON".into()),
    }
}

/// POST /api/shutdown (callers check the port is ours first).
pub fn request_shutdown(port: u16) -> Result<(), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    s.write_all(
        format!("POST /api/shutdown HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n").as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.take(64 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next().unwrap_or("");
    match line.split_whitespace().nth(1) {
        Some(c) if c.starts_with('2') => Ok(()),
        _ => Err(format!("its shutdown request answered {line:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_health_and_reads_the_version() {
        let ok = b"HTTP/1.1 200 OK\r\n\r\n{\"status\":\"ok\",\"name\":\"Personal Dev Team\",\"version\":\"0.1.7\"}";
        assert_eq!(classify_health(ok), Probe::Ours { version: "0.1.7".into() });
        assert!(matches!(classify_health(b"HTTP/1.1 500 X\r\n\r\n"), Probe::Foreign(_)));
        assert!(matches!(classify_health(b"HTTP/1.1 200 OK\r\n\r\n{\"name\":\"x\"}"), Probe::Foreign(_)));
        assert!(matches!(classify_health(b"junk"), Probe::Foreign(_)));
    }

    #[test]
    fn a_closed_port_is_free() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(probe(port), Probe::Free);
    }
}
