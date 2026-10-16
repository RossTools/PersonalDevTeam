//! A stand-in pdtd for pdt-host's tests. Reads `fake.json` next to its own exe: {"version": "...", "mode": "healthy" |
//! "crash" | "crash_once" | "unhealthy"}. Serves /api/health on $PDT_PORT, exits on POST /api/shutdown, and records
//! $PDT_SUPERVISED in `env.txt` beside itself.
use std::io::{Read, Write};

fn main() {
    let exe = std::env::current_exe().expect("exe");
    let dir = exe.parent().expect("dir").to_path_buf();
    let cfg: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("fake.json")).expect("fake.json")).expect("json");
    let version = cfg["version"].as_str().unwrap_or("0.0.0").to_string();
    let mode = cfg["mode"].as_str().unwrap_or("healthy").to_string();
    let _ = std::fs::write(dir.join("env.txt"), std::env::var("PDT_SUPERVISED").unwrap_or_default());
    let crash = mode == "crash" || (mode == "crash_once" && !dir.join("crashed-once").exists());
    if crash {
        let _ = std::fs::write(dir.join("crashed-once"), "");
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::process::exit(1);
    }
    let port: u16 = std::env::var("PDT_PORT").expect("PDT_PORT").parse().expect("port");
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind");
    for stream in listener.incoming() {
        let Ok(mut s) = stream else { continue };
        let mut buf = [0u8; 2048];
        let n = s.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        if req.starts_with("POST /api/shutdown") {
            let _ = s.write_all(b"HTTP/1.0 202 Accepted\r\nContent-Length: 0\r\n\r\n");
            let _ = s.flush();
            // Close gracefully: exiting with the socket open resets it on Windows and the reply is lost.
            let _ = s.shutdown(std::net::Shutdown::Write);
            drop(s);
            std::process::exit(0);
        }
        let reply = if req.starts_with("GET /api/health") && mode != "unhealthy" {
            let body = serde_json::json!({"status": "ok", "name": pdt_core::brand::PRODUCT_NAME, "version": version})
                .to_string();
            format!("HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
        } else {
            "HTTP/1.0 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_string()
        };
        let _ = s.write_all(reply.as_bytes());
    }
}
