//! A stand-in program for the workflow `command` action's tests, the same on every OS.
//!
//! - `echo [args…]`: prints `{"got": <stdin JSON>, "args": [...], "cwd": "<dir>", "env": $FAKE_ENV}`
//! - `message <text>`: prints `{"message": "<text>"}`
//! - `quiet`: prints `{"quiet": true}`
//! - `text`: prints `hello` (not JSON)
//! - `fail`: says `boom` on stderr and exits 3
//! - `sleep <ms>`: sleeps, then prints `{}`

use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("echo") => {
            let mut input = String::new();
            let _ = std::io::stdin().read_to_string(&mut input);
            let got: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
            let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            let out =
                serde_json::json!({"got": got, "args": &args[1..], "cwd": cwd, "env": std::env::var("FAKE_ENV").ok()});
            println!("{out}");
        }
        Some("message") => println!("{}", serde_json::json!({"message": args.get(1).cloned().unwrap_or_default()})),
        Some("quiet") => println!("{{\"quiet\": true}}"),
        Some("text") => println!("hello"),
        Some("fail") => {
            eprintln!("boom");
            std::process::exit(3);
        }
        Some("sleep") => {
            let ms = args.get(1).and_then(|m| m.parse().ok()).unwrap_or(1000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            println!("{{}}");
        }
        _ => {
            eprintln!("usage: fake-command echo|message|quiet|text|fail|sleep");
            std::process::exit(2);
        }
    }
}
