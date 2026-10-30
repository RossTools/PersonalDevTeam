//! A stand-in pdtd (and installer) for pdt-host's tests. Reads `fake.json` next to its own exe:
//! {"version": "...", "mode": "healthy" | "crash" | "crash_once" | "unhealthy" | "stubborn" | "die_soon",
//!  "installer": "hang" | "hang_tree" | "fail_partial", "migrate": bool}.
//! As pdtd: serves /api/health on $PDT_PORT and exits on POST /api/shutdown ("stubborn" answers but keeps running;
//! "die_soon" exits a second after starting). Beside itself it records $PDT_SUPERVISED in `env.txt`, every `PDT_*`
//! variable in `env.json` (and, as pdtd does, the record in `<PDT_DATA_DIR>/pdtd-env.json`), and the outcome in `<PDT_DATA_DIR>/update/result.json` at startup in `seen-result.txt`.
//! With "migrate" it stands in for a version with a new migration: at startup it rewrites `<PDT_DATA_DIR>/pdt.db`
//! (to `migrated by <version>`) and its `-wal` and `-shm` files.
//! As an installer (started with `/S /D=<dir>`): "hang" never finishes; "hang_tree" also starts a child that never
//! finishes (its pid in `grandchild.pid` beside the installer, written before the installer hangs); "fail_partial"
//! breaks `<dir>/fake.json` and fails; anything else (e.g. "noop") succeeds without changing anything.
use std::io::{Read, Write};

fn installer(cfg: &serde_json::Value, args: &[String]) -> ! {
    let dest = args.iter().find_map(|a| a.strip_prefix("/D=")).expect("/D=<dir>");
    if std::env::var_os("FAKE_GRANDCHILD").is_some() {
        let exe = std::env::current_exe().expect("exe");
        let _ = std::fs::write(exe.with_file_name("grandchild.pid"), std::process::id().to_string());
        std::thread::sleep(std::time::Duration::from_secs(3600));
        std::process::exit(0)
    }
    match cfg["installer"].as_str().unwrap_or("") {
        "hang_tree" => {
            let exe = std::env::current_exe().expect("exe");
            let pid_file = exe.with_file_name("grandchild.pid");
            let _ = std::process::Command::new(&exe).args(args).env("FAKE_GRANDCHILD", "1").spawn();
            // Hang only once the child has recorded its pid, so a timeout always finds the whole tree.
            while std::fs::read_to_string(&pid_file).map(|p| p.trim().is_empty()).unwrap_or(true) {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            std::thread::sleep(std::time::Duration::from_secs(3600));
            std::process::exit(0)
        }
        "hang" => {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            std::process::exit(0)
        }
        "fail_partial" => {
            let _ = std::fs::write(std::path::Path::new(dest).join("fake.json"), "{half-written");
            std::process::exit(3)
        }
        _ => std::process::exit(0),
    }
}

fn main() {
    let exe = std::env::current_exe().expect("exe");
    let dir = exe.parent().expect("dir").to_path_buf();
    let cfg: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("fake.json")).expect("fake.json")).expect("json");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "/S") {
        installer(&cfg, &args);
    }
    let version = cfg["version"].as_str().unwrap_or("0.0.0").to_string();
    let mode = cfg["mode"].as_str().unwrap_or("healthy").to_string();
    let _ = std::fs::write(dir.join("env.txt"), std::env::var("PDT_SUPERVISED").unwrap_or_default());
    let env: serde_json::Map<String, serde_json::Value> =
        std::env::vars().filter(|(k, _)| k.starts_with("PDT_")).map(|(k, v)| (k, v.into())).collect();
    let _ = std::fs::write(dir.join("env.json"), serde_json::Value::Object(env).to_string());
    if let Ok(d) = std::env::var("PDT_DATA_DIR") {
        let started =
            pdt_core::handoff::StartedEnv::new(&version, std::env::vars().filter(|(k, _)| k.starts_with("PDT_")));
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(
            pdt_core::handoff::started_env_path(std::path::Path::new(&d)),
            serde_json::to_vec(&started).expect("json"),
        );
    }
    let seen = std::env::var("PDT_DATA_DIR")
        .ok()
        .and_then(|d| std::fs::read(std::path::Path::new(&d).join("update").join("result.json")).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["outcome"].as_str().map(String::from))
        .unwrap_or_else(|| "none".into());
    let _ = std::fs::write(dir.join("seen-result.txt"), seen);
    if cfg["migrate"].as_bool() == Some(true)
        && let Ok(d) = std::env::var("PDT_DATA_DIR")
    {
        let db = std::path::Path::new(&d).join(pdt_core::handoff::DB_FILE);
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(&db, format!("migrated by {version}"));
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::write(format!("{}{suffix}", db.display()), "new");
        }
    }
    let crash = mode == "crash" || (mode == "crash_once" && !dir.join("crashed-once").exists());
    if crash {
        let _ = std::fs::write(dir.join("crashed-once"), "");
        std::thread::sleep(std::time::Duration::from_millis(200));
        std::process::exit(1);
    }
    if mode == "die_soon" {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(1));
            std::process::exit(1);
        });
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
            if mode != "stubborn" {
                std::process::exit(0);
            }
            continue;
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
