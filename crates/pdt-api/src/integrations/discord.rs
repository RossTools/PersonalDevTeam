//! Discord (PDT-98): webhooks that Post to Discord steps post to, and a bot the PM manages a server with (its tools
//! and the `discord.channel.ensure` action).

use super::{Integration, Tool};
use crate::mcp::tools::{Cx, args, confirmable, schema};
use crate::workflow::catalog::{BoxFut, NodeKind};
use crate::workflow::nodes::discord::DISCORD_WEBHOOK;
use pdt_core::discord::DISCORD_BOT;
use pdt_core::integration::{ConnectionKind, ToolDecl};
use pdt_core::pm::AskTarget;
use serde_json::Value;

pub struct Discord;

impl Integration for Discord {
    fn id(&self) -> &'static str {
        "discord"
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![
            ConnectionKind {
                id: DISCORD_WEBHOOK.into(),
                label: "Discord webhook".into(),
                plural: "Discord webhooks".into(),
                setup: "A channel that Post to Discord steps post to. In Discord, open the channel’s settings → \
                        Integrations → Webhooks → New Webhook, copy its URL, and add it here."
                    .into(),
                description: "Posts to one Discord channel (Post to Discord).".into(),
                url: false,
                secret_label: "Webhook URL".into(),
                secret_hint: "In Discord: the channel’s settings (Edit Channel) → Integrations → Webhooks → New \
                              Webhook → Copy Webhook URL. It looks like https://discord.com/api/webhooks/…"
                    .into(),
                ..Default::default()
            },
            ConnectionKind {
                id: DISCORD_BOT.into(),
                label: "Discord bot".into(),
                plural: "Discord bots".into(),
                setup: "A bot that lets the PM manage a Discord server: make and organise channels, and make the \
                        webhooks workflows post to. In the Discord Developer Portal, make an application, then under \
                        Bot reset its token and copy it. Invite the bot to your server with the link this dialog \
                        shows (Manage Channels, Manage Webhooks, Send Messages)."
                    .into(),
                description: "Lets the PM manage a Discord server’s channels and webhooks.".into(),
                url: false,
                secret_label: "Bot token".into(),
                secret_hint: "Discord Developer Portal → your application → Bot → Reset Token → Copy.".into(),
                discord_bot: true,
                pm_acts: Some("The PM changes this server without asking".into()),
                ..Default::default()
            },
        ]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(crate::workflow::nodes::discord::Discord), Box::new(crate::discord::EnsureChannel)]
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                decl: ToolDecl::new("discord_get", "Look at the Discord server: its channels and webhooks")
                    .read_only()
                    .open_world()
                    .pm(),
                title: "Look at the Discord server",
                description: "The Discord server a bot connection manages: its categories, channels and webhooks (with \
                              the connection each webhook is saved as), and whether you may change it without asking.",
                schema: schema::<crate::discord::DiscordGet>,
            },
            Tool {
                decl: ToolDecl::new("discord_change", "Create, rename, move or archive Discord channels")
                    .asks()
                    .open_world()
                    .pm(),
                title: "Change the Discord server",
                description: "Creates, renames, moves or archives (moves into Archive) channels and categories on the \
                              Discord server, in order. The user confirms each call unless the bot connection lets you \
                              act.",
                schema: schema::<crate::discord::DiscordChange>,
            },
            Tool {
                decl: ToolDecl::new("discord_webhook", "Make a Discord webhook for discord.post steps")
                    .asks()
                    .open_world()
                    .pm(),
                title: "Make a Discord webhook",
                description: "Makes a webhook on a channel (and the channel, with createChannel) and saves it as a \
                              discord_webhook connection that discord.post steps use by name. Its URL is never shown. \
                              The user confirms each call unless the bot connection lets you act.",
                schema: schema::<crate::discord::DiscordWebhookArgs>,
            },
        ]
    }

    fn call_tool<'a>(&'a self, cx: &'a Cx<'a>, name: &'a str, a: Value) -> BoxFut<'a, Result<Value, String>> {
        let st = cx.st;
        Box::pin(async move {
            match name {
                "discord_get" => crate::discord::get(st, args(a)?).await,
                "discord_change" => {
                    let a: crate::discord::DiscordChange = args(a)?;
                    let target = AskTarget::Connection { kind: DISCORD_BOT, name: a.connection.clone() };
                    confirmable(cx, name, &target).await?;
                    crate::discord::change(st, a).await
                }
                "discord_webhook" => {
                    let a: crate::discord::DiscordWebhookArgs = args(a)?;
                    let target = AskTarget::Connection { kind: DISCORD_BOT, name: a.connection.clone() };
                    confirmable(cx, name, &target).await?;
                    crate::discord::webhook_tool(st, a).await
                }
                other => Err(format!("no tool is called {other}")),
            }
        })
    }
}
