//! The bot's Gateway connection (PDT-304): a websocket from pdtd to Discord (outbound only: no public URL, no
//! Interactions endpoint) that gets the bot's DMs, button presses and slash commands. It sends heartbeats, resumes
//! after a dropped connection (else identifies again), and backs off between tries; a wrong token or a refused intent
//! stops it with a message for the user.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

/// The DMs and their text: `DIRECT_MESSAGES` and `MESSAGE_CONTENT`.
pub const INTENTS: u64 = (1 << 12) | (1 << 15);
/// The Gateway's API version and encoding, added to its URL.
const QUERY: &str = "v=10&encoding=json";
/// The longest wait between tries.
const MAX_BACKOFF: Duration = Duration::from_secs(120);

/// One event the Gateway dispatched (`t` and `d`).
#[derive(Debug, Clone)]
pub struct Dispatch {
    pub name: String,
    pub data: Value,
}

/// What the link reports as it goes.
#[derive(Debug, Clone)]
pub enum Report {
    /// Connected and identified (or resumed).
    Connected,
    /// Lost; trying again.
    Retrying(String),
    /// Stopped for good: the user has to change something.
    Failed(String),
    Event(Dispatch),
}

/// What resuming a session needs.
#[derive(Debug, Clone, Default)]
struct Resume {
    session_id: String,
    url: String,
    seq: Option<u64>,
}

/// Why a session ended.
#[derive(Debug)]
enum End {
    /// Connect again, resuming when it can.
    Again { resume: bool, why: String },
    /// Don't: the user must fix it.
    Fatal(String),
}

/// Why a Gateway close code stops the connection for good (the user must change something); None: try again.
pub fn close_fatal(code: u16) -> Option<&'static str> {
    match code {
        4004 => Some("Discord refused the bot token: set the connection's token again"),
        4010..=4012 => Some("Discord refused the bot's connection settings"),
        4013 => Some("Discord refused the bot's intents"),
        4014 => Some(
            "Discord refused the Message Content intent: in the Developer Portal, open the application → Bot → \
             Privileged Gateway Intents and turn on Message Content Intent",
        ),
        _ => None,
    }
}

/// Whether a session closed with `code` can be resumed.
pub fn close_resumable(code: u16) -> bool {
    !matches!(code, 4007 | 4009)
}

pub fn identify(token: &str) -> Value {
    json!({"op": 2, "d": {"token": token, "intents": INTENTS,
           "properties": {"os": std::env::consts::OS, "browser": pdt_core::brand::PRODUCT_NAME,
                          "device": pdt_core::brand::PRODUCT_NAME}}})
}

fn resume_payload(token: &str, r: &Resume) -> Value {
    json!({"op": 6, "d": {"token": token, "session_id": r.session_id, "seq": r.seq}})
}

fn heartbeat(seq: Option<u64>) -> Message {
    Message::Text(json!({"op": 1, "d": seq}).to_string().into())
}

/// `url` with the Gateway's query.
pub fn with_query(url: &str) -> String {
    let base = url.trim_end_matches('/');
    if base.contains('?') { format!("{base}&{QUERY}") } else { format!("{base}/?{QUERY}") }
}

/// The wait before try `n` (1-based): 1 s, doubling, at most [`MAX_BACKOFF`].
pub fn backoff(n: u32) -> Duration {
    Duration::from_secs(1u64 << n.saturating_sub(1).min(7)).min(MAX_BACKOFF)
}

/// Holds the Gateway connection until `reports` is dropped or a fatal close: `url` is the Gateway's (from
/// `GET /gateway/bot`).
pub async fn run(url: String, token: String, reports: mpsc::Sender<Report>) {
    let mut resume: Option<Resume> = None;
    let mut tries = 0u32;
    loop {
        let target = resume.as_ref().map_or_else(|| url.clone(), |r| r.url.clone());
        let end = session(&with_query(&target), &token, &mut resume, &reports, &mut tries).await;
        if reports.is_closed() {
            return;
        }
        match end {
            End::Fatal(why) => {
                tracing::warn!(error = %why, "the Discord bot's Gateway connection stopped");
                let _ = reports.send(Report::Failed(why)).await;
                return;
            }
            End::Again { resume: keep, why } => {
                if !keep {
                    resume = None;
                }
                tries += 1;
                let wait = backoff(tries);
                tracing::info!(why = %why, wait_s = wait.as_secs(), "the Discord bot's Gateway connection dropped");
                let _ = reports.send(Report::Retrying(why)).await;
                tokio::time::sleep(wait).await;
            }
        }
    }
}

/// One connection: Hello, identify or resume, then heartbeats and dispatches until it ends.
async fn session(
    url: &str,
    token: &str,
    resume: &mut Option<Resume>,
    reports: &mpsc::Sender<Report>,
    tries: &mut u32,
) -> End {
    let again = |why: String| End::Again { resume: true, why };
    let (mut ws, _) = match tokio_tungstenite::connect_async(url).await {
        Ok(c) => c,
        Err(e) => return again(format!("could not reach Discord's Gateway: {e}")),
    };
    // Hello: how often to beat.
    let hello = match tokio::time::timeout(Duration::from_secs(30), ws.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<Value>(&t).unwrap_or(Value::Null),
        _ => return again("Discord's Gateway said no hello".into()),
    };
    let every = hello["d"]["heartbeat_interval"].as_u64().filter(|_| hello["op"] == 10).unwrap_or(41_250);
    let first = match resume.as_ref() {
        Some(r) => resume_payload(token, r),
        None => identify(token),
    };
    if ws.send(Message::Text(first.to_string().into())).await.is_err() {
        return again("the Gateway closed before PDT identified".into());
    }
    let mut seq: Option<u64> = resume.as_ref().and_then(|r| r.seq);
    // The first beat comes at a random point of the interval (Discord asks for jitter), the rest every interval.
    let jitter = Duration::from_millis(every.saturating_mul(u64::from(rand_percent())) / 100);
    let mut beat = tokio::time::interval_at(tokio::time::Instant::now() + jitter, Duration::from_millis(every));
    let mut acked = true;
    loop {
        tokio::select! {
            _ = beat.tick() => {
                if !acked {
                    let _ = ws.close(Some(CloseFrame { code: 4000.into(), reason: "no heartbeat ack".into() })).await;
                    return again("Discord stopped answering heartbeats".into());
                }
                acked = false;
                if ws.send(heartbeat(seq)).await.is_err() {
                    return again("the Gateway connection broke".into());
                }
            }
            _ = reports.closed() => {
                let _ = ws.close(None).await;
                return End::Fatal("stopped".into());
            }
            frame = ws.next() => {
                let text = match frame {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(c))) => {
                        let code = c.as_ref().map(|c| u16::from(c.code)).unwrap_or(1000);
                        if let Some(why) = close_fatal(code) {
                            return End::Fatal(why.into());
                        }
                        let reason = c.map(|c| c.reason.to_string()).unwrap_or_default();
                        return End::Again { resume: close_resumable(code), why: format!("Discord closed it ({code} {reason})") };
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return again(format!("the Gateway connection broke: {e}")),
                    None => return again("the Gateway connection ended".into()),
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(s) = v["s"].as_u64() {
                    seq = Some(s);
                    if let Some(r) = resume.as_mut() {
                        r.seq = seq;
                    }
                }
                match v["op"].as_u64() {
                    Some(0) => {
                        let name = v["t"].as_str().unwrap_or_default().to_string();
                        if name == "READY" {
                            let d = &v["d"];
                            *resume = Some(Resume {
                                session_id: d["session_id"].as_str().unwrap_or_default().into(),
                                url: d["resume_gateway_url"].as_str().map(String::from).unwrap_or_else(|| url_base(url)),
                                seq,
                            });
                        }
                        if name == "READY" || name == "RESUMED" {
                            *tries = 0;
                            let _ = reports.send(Report::Connected).await;
                        }
                        if reports.send(Report::Event(Dispatch { name, data: v["d"].clone() })).await.is_err() {
                            return End::Fatal("stopped".into());
                        }
                    }
                    Some(1) => {
                        if ws.send(heartbeat(seq)).await.is_err() {
                            return again("the Gateway connection broke".into());
                        }
                    }
                    Some(7) => {
                        let _ = ws.close(Some(CloseFrame { code: 4000.into(), reason: "reconnect".into() })).await;
                        return again("Discord asked PDT to reconnect".into());
                    }
                    Some(9) => {
                        let keep = v["d"].as_bool() == Some(true);
                        let _ = ws.close(None).await;
                        // Discord asks for a short random wait before identifying again.
                        tokio::time::sleep(Duration::from_millis(1000 + u64::from(rand_percent()) * 40)).await;
                        return End::Again { resume: keep, why: "Discord ended the session".into() };
                    }
                    Some(11) => acked = true,
                    _ => {}
                }
            }
        }
    }
}

/// The URL without its query (a resume URL Discord did not give).
fn url_base(url: &str) -> String {
    url.split('?').next().unwrap_or(url).trim_end_matches('/').to_string()
}

/// 0..100, from the clock (jitter needs nothing better).
fn rand_percent() -> u8 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    (n % 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_asks_for_dms_and_their_text() {
        let v = identify("tok");
        assert_eq!((v["op"].as_u64(), v["d"]["token"].as_str()), (Some(2), Some("tok")));
        assert_eq!(v["d"]["intents"], 4096 + 32768);
    }

    #[test]
    fn a_refused_token_or_intent_stops_it_and_says_why() {
        assert!(close_fatal(4004).unwrap().contains("token"));
        assert!(close_fatal(4014).unwrap().contains("Message Content Intent"));
        assert_eq!(close_fatal(4000), None);
        assert_eq!(close_fatal(1001), None);
        assert!(close_resumable(4000) && close_resumable(1006));
        assert!(!close_resumable(4007) && !close_resumable(4009));
    }

    #[test]
    fn urls_get_the_version_and_tries_back_off() {
        assert_eq!(with_query("wss://gateway.discord.gg"), "wss://gateway.discord.gg/?v=10&encoding=json");
        assert_eq!(with_query("ws://127.0.0.1:9/ws?x=1"), "ws://127.0.0.1:9/ws?x=1&v=10&encoding=json");
        assert_eq!(url_base("wss://a.b/?v=10"), "wss://a.b");
        let waits: Vec<u64> = (1..=10).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 64, 120, 120, 120]);
    }
}
