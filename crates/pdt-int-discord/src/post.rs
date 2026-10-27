//! `discord.post` (Phase 7e spec §3.3): posts to a Discord channel through a webhook (a `discord_webhook` connection
//! whose secret is the webhook URL). Long messages are split at line breaks; a rate limit waits and tries again. Each
//! piece posted is recorded as it goes, so a restart or a retry posts only the rest.

use pdt_api::sdk::http::{client, read_body, send_error};
use pdt_api::sdk::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, spec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

pub const DISCORD_WEBHOOK: &str = "discord_webhook";

/// Discord's limit on one message's content.
pub const MAX_CONTENT: usize = 2000;

/// How many times a rate-limited post is tried.
const TRIES: u32 = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DiscordConfig {
    /// A `discord_webhook` connection (its secret is the webhook URL).
    #[schemars(extend("x-connection-kind" = "discord_webhook"))]
    pub connection: String,
    /// The message (Discord Markdown). Longer than 2000 characters, it goes as several messages.
    #[serde(default)]
    pub content: String,
    /// Shown as the sender instead of the webhook's name.
    #[serde(default)]
    pub username: Option<String>,
    /// Discord embeds, as Discord's JSON (sent with the first message).
    #[serde(default)]
    #[ts(type = "unknown[]")]
    pub embeds: Vec<Value>,
    /// A thread to post in (its id).
    #[serde(default)]
    pub thread: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiscordOutput {
    pub message_ids: Vec<String>,
}

/// `content` in pieces of at most `max` characters, cut at line breaks where it can (else at spaces, else anywhere).
pub fn split_content(content: &str, max: usize) -> Vec<String> {
    let mut out = vec![];
    let mut rest = content.trim_end();
    while rest.chars().count() > max {
        let limit = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
        let head = &rest[..limit];
        let cut = head.rfind('\n').or_else(|| head.rfind(' ')).filter(|&i| i > 0).unwrap_or(limit);
        out.push(rest[..cut].trim_end().to_string());
        rest = rest[cut..].trim_start_matches(['\n', ' ']);
    }
    if !rest.is_empty() || out.is_empty() {
        out.push(rest.to_string());
    }
    out
}

pub struct Discord;

impl NodeKind for Discord {
    fn spec(&self) -> KindSpec {
        spec::<DiscordConfig, DiscordOutput>(
            "discord.post",
            1,
            Category::Action,
            "Post to Discord",
            "Posts a message to a Discord channel through a webhook connection (kind discord_webhook, its secret the \
             webhook URL).",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: DiscordConfig = parse(config)?;
        if c.connection.trim().is_empty() {
            return Err("connection is empty: name a discord_webhook connection".into());
        }
        if c.content.trim().is_empty() && c.embeds.is_empty() {
            return Err("give content or embeds".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: DiscordConfig = parse(&input)?;
            let pieces =
                if c.content.trim().is_empty() { vec![String::new()] } else { split_content(&c.content, MAX_CONTENT) };
            if cx.dry_run {
                let what = format!("post {} message(s) to Discord via {}: {}", pieces.len(), c.connection, c.content);
                return Ok(Outcome::next("out", json!({"would": what})));
            }
            let (conn, secret) = pdt_api::sdk::connections::connection_with_secret(cx.st, &c.connection).await?;
            if conn.kind != DISCORD_WEBHOOK {
                return Err(format!("{} is a {} connection, not {DISCORD_WEBHOOK}", conn.name, conn.kind));
            }
            let url =
                secret.filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("{} has no webhook URL", conn.name))?;
            // Pieces posted before a restart or a failed try are not posted again (PDT-131): each one's id is saved as
            // it goes.
            let mut ids: Vec<String> = cx
                .step
                .output
                .get("messageIds")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            for (i, content) in pieces.iter().enumerate().skip(ids.len()) {
                let mut body = json!({"content": content, "allowed_mentions": {"parse": []}});
                if let Some(u) = &c.username {
                    body["username"] = json!(u);
                }
                if i == 0 && !c.embeds.is_empty() {
                    body["embeds"] = json!(c.embeds);
                }
                ids.push(post(&url, c.thread.as_deref(), &body).await?);
                cx.save_progress(&json!(DiscordOutput { message_ids: ids.clone() })).await?;
            }
            Ok(Outcome::next("out", DiscordOutput { message_ids: ids }))
        })
    }
}

/// One post; returns the message id. A 429 waits Discord's `retry_after` and tries again.
async fn post(url: &str, thread: Option<&str>, body: &Value) -> Result<String, String> {
    let mut query = vec![("wait", "true".to_string())];
    if let Some(t) = thread {
        query.push(("thread_id", t.to_string()));
    }
    for _ in 0..TRIES {
        let resp = client()
            .post(url)
            .query(&query)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Discord: {}", send_error(e)))?;
        let status = resp.status().as_u16();
        let bytes = read_body(resp, 64 * 1024).await?;
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        match status {
            200..=299 => return Ok(v["id"].as_str().unwrap_or_default().to_string()),
            429 => {
                let secs = v["retry_after"].as_f64().unwrap_or(1.0).clamp(0.0, 60.0);
                tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
            }
            _ => {
                let msg = v["message"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
                return Err(format!("Discord answered {status}: {msg}"));
            }
        }
    }
    Err("Discord kept rate-limiting the post".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_split_at_line_breaks() {
        assert_eq!(split_content("hi", 10), vec!["hi"]);
        assert_eq!(split_content("aaaa\nbbbb\ncccc", 10), vec!["aaaa\nbbbb", "cccc"]);
        assert_eq!(split_content("aaaa bbbb cccc", 10), vec!["aaaa bbbb", "cccc"]);
        assert_eq!(split_content("abcdefghijkl", 5), vec!["abcde", "fghij", "kl"]);
        let long = "line\n".repeat(1000);
        let parts = split_content(&long, MAX_CONTENT);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_CONTENT));
        assert_eq!(parts.concat().matches("line").count(), 1000);
        assert_eq!(split_content("ééééé", 2), vec!["éé", "éé", "é"], "characters, not bytes");
    }
}
