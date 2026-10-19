//! `pdtd hook …` (remote-host spec §5.1, decision 11): reports an agent hook event to the pdtd that owns the session
//! (`PDT_SESSION_ID`, `PDT_HOOK_PORT`). It never writes to stdout (Claude adds UserPromptSubmit stdout to the
//! conversation) and always exits 0, quickly, whatever happens.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const STDIN_LIMIT: u64 = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(2);
/// Codex notifies twice per user turn (Task 1); the second call is its internal title-generation thread, whose
/// first input message starts with this.
const CODEX_TITLE_PROMPT: &str = "Generate a concise, single-line task title";

pub fn run(args: &[String], stdin: &mut dyn Read, env: &dyn Fn(&str) -> Option<String>) -> i32 {
    let report = match args.first().map(String::as_str) {
        Some("claude") => {
            let Some(event) = args.get(1) else { return 0 };
            let mut buf = Vec::new();
            let _ = stdin.take(STDIN_LIMIT).read_to_end(&mut buf);
            let input: serde_json::Value = serde_json::from_slice(&buf).unwrap_or_default();
            serde_json::json!({
                "source": "claude",
                "event": event,
                "notificationType": input.get("notification_type").and_then(|v| v.as_str()),
                "sessionId": input.get("session_id").and_then(|v| v.as_str()),
                "startSource": (event == "SessionStart").then(|| input.get("source").and_then(|v| v.as_str())).flatten(),
            })
        }
        Some("codex-notify") => {
            let (chain, payload) = split_chain(&args[1..]);
            if let Some(p) = payload
                && !chain.is_empty()
            {
                run_chain(&chain, p);
            }
            let payload: serde_json::Value = payload.and_then(|p| serde_json::from_str(p).ok()).unwrap_or_default();
            if is_codex_title_thread(&payload) {
                return 0;
            }
            let event = payload.get("type").and_then(|t| t.as_str()).unwrap_or_default();
            serde_json::json!({ "source": "codex", "event": event })
        }
        _ => return 0,
    };
    if let (Some(id), Some(port)) = (env("PDT_SESSION_ID"), env("PDT_HOOK_PORT").and_then(|p| p.parse::<u16>().ok())) {
        let _ = post(port, &format!("/api/sessions/{id}/activity"), &report.to_string());
    }
    0
}

/// Whether a Codex notify payload is for Codex's own title-generation thread rather than the user's turn.
fn is_codex_title_thread(payload: &serde_json::Value) -> bool {
    payload
        .get("input-messages")
        .and_then(|m| m.get(0))
        .and_then(|m| m.as_str())
        .is_some_and(|m| m.trim_start().starts_with(CODEX_TITLE_PROMPT))
}

/// `[--chain <cmd…> --] <payload>` → (cmd, payload). Codex appends the payload as the last argument.
fn split_chain(args: &[String]) -> (Vec<String>, Option<&str>) {
    let payload = args.last().map(String::as_str);
    if args.first().map(String::as_str) != Some("--chain") || args.len() < 2 {
        return (Vec::new(), payload);
    }
    match args[..args.len() - 1].iter().rposition(|a| a == "--") {
        Some(end) if end > 1 => (args[1..end].to_vec(), payload),
        _ => (Vec::new(), payload),
    }
}

/// Starts the user's own notifier with the same payload, without waiting for it.
fn run_chain(chain: &[String], payload: &str) {
    let mut cmd = std::process::Command::new(&chain[0]);
    cmd.args(&chain[1..])
        .arg(payload)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let _ = cmd.spawn();
}

fn post(port: u16, path: &str, body: &str) -> std::io::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    let req = format!(
        "POST {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes())?;
    let mut sink = [0u8; 1024];
    let _ = s.read(&mut sink);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn chain_parsing() {
        assert_eq!(split_chain(&s(&["{}"])), (vec![], Some("{}")));
        assert_eq!(
            split_chain(&s(&["--chain", "n.exe", "turn-ended", "--", "{}"])),
            (s(&["n.exe", "turn-ended"]), Some("{}"))
        );
        assert_eq!(split_chain(&s(&["--chain", "a", "--", "b", "--", "{}"])), (s(&["a", "--", "b"]), Some("{}")));
        assert_eq!(split_chain(&s(&["--chain", "a", "{}"])), (vec![], Some("{}")));
        assert_eq!(split_chain(&[]), (vec![], None));
    }

    #[test]
    fn unknown_or_incomplete_input_exits_0_without_network() {
        let env = |_: &str| None;
        assert_eq!(run(&s(&["claude"]), &mut std::io::empty(), &env), 0);
        assert_eq!(run(&s(&["what"]), &mut std::io::empty(), &env), 0);
        assert_eq!(run(&s(&["claude", "Stop"]), &mut "not json".as_bytes(), &env), 0);
    }

    /// Runs `pdtd hook codex-notify <payload>` against a listener standing in for pdtd; returns the request it got.
    fn notify(payload: &str) -> Option<String> {
        report(&s(&["codex-notify", payload]), "")
    }

    /// Runs `pdtd hook <args>` with `stdin` against a listener standing in for pdtd; returns the request it got.
    fn report(args: &[String], stdin: &str) -> Option<String> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let env = move |k: &str| match k {
            "PDT_SESSION_ID" => Some("s1".to_string()),
            "PDT_HOOK_PORT" => Some(port.clone()),
            _ => None,
        };
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if let Ok((mut conn, _)) = listener.accept() {
                    conn.set_nonblocking(false).unwrap();
                    conn.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut buf = vec![0u8; 4096];
                    let n = conn.read(&mut buf).unwrap_or(0);
                    let _ = conn.write_all(b"HTTP/1.0 204 No Content\r\n\r\n");
                    return Some(String::from_utf8_lossy(&buf[..n]).into_owned());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            None
        });
        assert_eq!(run(args, &mut stdin.as_bytes(), &env), 0);
        server.join().unwrap()
    }

    #[test]
    fn claude_reports_its_conversation_id_and_why_a_session_started() {
        let input = r#"{"session_id":"new-conv","source":"clear","hook_event_name":"SessionStart"}"#;
        let req = report(&s(&["claude", "SessionStart"]), input).expect("reported");
        let body: serde_json::Value = serde_json::from_str(req.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["sessionId"], "new-conv");
        assert_eq!(body["startSource"], "clear");
        let input = r#"{"session_id":"c","source":"elsewhere"}"#;
        let req = report(&s(&["claude", "Stop"]), input).expect("reported");
        let body: serde_json::Value = serde_json::from_str(req.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(body["startSource"].is_null(), "only SessionStart has a start source");
    }

    #[test]
    fn codex_turns_are_reported_but_its_title_thread_is_not() {
        let turn = r#"{"type":"agent-turn-complete","thread-id":"a","input-messages":["reply with ok"],"last-assistant-message":"ok"}"#;
        let req = notify(turn).expect("a user turn is reported");
        assert!(req.starts_with("POST /api/sessions/s1/activity "), "{req}");
        let body: serde_json::Value = serde_json::from_str(req.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body, serde_json::json!({"source": "codex", "event": "agent-turn-complete"}));
        // Task 1: Codex notifies a second time per turn, for its internal title-generation thread.
        let title = r#"{"type":"agent-turn-complete","thread-id":"b","input-messages":["Generate a concise, single-line task title for this conversation..."],"last-assistant-message":"{\"title\":\"Reply ok\"}"}"#;
        assert_eq!(notify(title), None, "the title thread's notify is ignored");
    }
}
