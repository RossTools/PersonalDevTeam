//! Discord (PDT-98): webhooks that Post to Discord steps post to, and a bot the PM manages a server with (its tools
//! and the `discord.channel.ensure` action).

use super::Integration;
use super::action::AnyAction;
use crate::workflow::catalog::NodeKind;
use crate::workflow::nodes::discord::DISCORD_WEBHOOK;
use pdt_core::discord::DISCORD_BOT;
use pdt_core::integration::ConnectionKind;
use serde_json::Value;
use std::sync::Arc;

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

    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        crate::discord::actions()
    }

    fn approval_title(&self, tool: &str, input: &Value) -> Option<String> {
        matches!(tool, "discord_change" | "discord_webhook")
            .then(|| pdt_core::discord::ask_title(tool, input).unwrap_or_else(|| "Discord".into()))
    }

    fn pm_prompt(&self) -> Option<&'static str> {
        Some(PM_PROMPT)
    }
}

/// What the PM's prompt says about the Discord tools.
pub(super) const PM_PROMPT: &str = "- Discord: `discord_get` shows the server (channels, categories, webhooks). `discord_change` \
creates, renames, moves and archives channels and categories; `discord_webhook` makes a channel's webhook and saves it \
as a `discord_webhook` connection that `discord.post` steps use by name (its URL never reaches you). Both wait for the \
user's yes unless the bot connection lets you act; say what you will change first.";
