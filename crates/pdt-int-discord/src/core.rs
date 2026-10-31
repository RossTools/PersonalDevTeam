//! A Discord server through a bot (PDT-98): the `discord_bot` connection's config, channel names, finding a channel
//! by id or name, and the changes the PM's `discord_change` tool makes. Pure: pdt-api calls Discord's REST API.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The connection kind: its secret is the bot's token, `config.guildId` the server.
pub const DISCORD_BOT: &str = "discord_bot";

/// Discord's REST API, used when the connection has no URL (tests give one).
pub const API_BASE: &str = "https://discord.com/api/v10";

/// Where `archive` moves a channel.
pub const ARCHIVE_CATEGORY: &str = "Archive";

const MAX_NAME: usize = 100;

/// A `discord_bot` connection's config: the server it manages, whether the PM may change it without asking, and who
/// may talk to the PM through the bot's DMs (PDT-304).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BotConfig {
    pub guild_id: String,
    pub pm_acts: bool,
    /// The user's Discord user id (`config.dmUserId`): set, the bot holds a Gateway connection and passes their DMs to
    /// the PM. Nobody else's are read.
    pub dm_user_id: Option<String>,
    /// Inbox items are sent as DMs too (`config.dmInbox`).
    pub dm_inbox: bool,
}

/// Checks a `discord_bot` connection's config (`guildId`: the server's id, digits; `dmUserId`, when given, a user's).
pub fn bot_config(config: &Value) -> Result<BotConfig, String> {
    let guild = config.get("guildId").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if guild.is_empty() {
        return Err("a Discord bot connection needs the server's id (config.guildId)".into());
    }
    if !guild.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{guild} is not a Discord server id (it is all digits)"));
    }
    let user = config.get("dmUserId").and_then(Value::as_str).map(str::trim).filter(|u| !u.is_empty());
    if let Some(u) = user
        && !u.chars().all(|c| c.is_ascii_digit())
    {
        return Err(format!("{u} is not a Discord user id (it is all digits)"));
    }
    Ok(BotConfig {
        guild_id: guild.into(),
        pm_acts: config.get("pmActs").and_then(Value::as_bool) == Some(true),
        dm_user_id: user.map(String::from),
        dm_inbox: user.is_some() && config.get("dmInbox").and_then(Value::as_bool) == Some(true),
    })
}

/// What a channel is (Discord's channel types, the ones a server has).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    #[default]
    Text,
    Voice,
    Category,
    Announcement,
    Forum,
    Other,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChannelKind::Text => "text",
            ChannelKind::Voice => "voice",
            ChannelKind::Category => "category",
            ChannelKind::Announcement => "announcement",
            ChannelKind::Forum => "forum",
            ChannelKind::Other => "other",
        }
    }
    /// From Discord's `type`.
    pub fn from_type(t: u64) -> Self {
        match t {
            0 => ChannelKind::Text,
            2 => ChannelKind::Voice,
            4 => ChannelKind::Category,
            5 => ChannelKind::Announcement,
            15 => ChannelKind::Forum,
            _ => ChannelKind::Other,
        }
    }
    /// Discord's `type` (None: PDT doesn't make this kind).
    pub fn to_type(self) -> Option<u64> {
        match self {
            ChannelKind::Text => Some(0),
            ChannelKind::Voice => Some(2),
            ChannelKind::Category => Some(4),
            ChannelKind::Announcement => Some(5),
            ChannelKind::Forum => Some(15),
            ChannelKind::Other => None,
        }
    }
}

/// A channel or category of the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    /// Its category's id.
    pub parent_id: Option<String>,
    pub position: i64,
    pub topic: Option<String>,
}

impl Channel {
    /// From Discord's channel object.
    pub fn from_api(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(String::from);
        Some(Channel {
            id: s("id")?,
            name: s("name").unwrap_or_default(),
            kind: ChannelKind::from_type(v.get("type").and_then(Value::as_u64).unwrap_or(0)),
            parent_id: s("parent_id"),
            position: v.get("position").and_then(Value::as_i64).unwrap_or(0),
            topic: s("topic").filter(|t| !t.is_empty()),
        })
    }
}

/// A name for a new or renamed channel. Text-like channels are lower case with dashes (as Discord makes them, `#`
/// dropped); categories and voice channels keep the name as given.
pub fn valid_name(name: &str, kind: ChannelKind) -> Result<String, String> {
    let n = name.trim();
    let n = match kind {
        ChannelKind::Category | ChannelKind::Voice => n.to_string(),
        _ => n.trim_start_matches('#').split_whitespace().collect::<Vec<_>>().join("-").to_lowercase(),
    };
    if n.is_empty() || n.chars().count() > MAX_NAME || n.contains('\n') {
        return Err(format!("{name:?} is not a channel name (1 to {MAX_NAME} characters)"));
    }
    Ok(n)
}

/// The channel (`categories`: or the category) that `given` means: its id, its name, or `#name`.
pub fn find<'a>(channels: &'a [Channel], given: &str, categories: bool) -> Result<&'a Channel, String> {
    let g = given.trim();
    let what = if categories { "category" } else { "channel" };
    let pool = || channels.iter().filter(|c| (c.kind == ChannelKind::Category) == categories);
    if let Some(c) = pool().find(|c| c.id == g) {
        return Ok(c);
    }
    let bare = g.trim_start_matches('#');
    let dashed = bare.split_whitespace().collect::<Vec<_>>().join("-");
    let named: Vec<&Channel> =
        pool().filter(|c| c.name.eq_ignore_ascii_case(bare) || c.name.eq_ignore_ascii_case(&dashed)).collect();
    match named.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("the server has no {what} called {g}")),
        many => Err(format!(
            "{} {what}s are called {g}; give one's id: {}",
            many.len(),
            many.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// One change `discord_change` makes. Channels and categories are named by id, name or `#name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    /// A new channel or category.
    Create {
        name: String,
        /// text (the default), category, voice, announcement or forum.
        #[serde(default)]
        kind: ChannelKind,
        /// The category to put it in (not for a category).
        #[serde(default)]
        category: Option<String>,
        /// A text channel's topic.
        #[serde(default)]
        topic: Option<String>,
    },
    /// Renames a channel or category.
    Rename { channel: String, name: String },
    /// Moves a channel into a category; out of any when `category` is left out.
    Move {
        channel: String,
        #[serde(default)]
        category: Option<String>,
    },
    /// Moves a channel into the Archive category (made when missing). Nothing is deleted.
    Archive { channel: String },
}

/// `#name` for a text-like channel, the name as it is for a category.
fn shown(name: &str) -> String {
    let n = name.trim();
    if n.starts_with('#') { n.to_string() } else { format!("#{n}") }
}

impl Change {
    /// One line for the user's yes ("create #ai-briefs in Bots").
    pub fn describe(&self) -> String {
        match self {
            Change::Create { name, kind: ChannelKind::Category, .. } => format!("create category {}", name.trim()),
            Change::Create { name, kind, category, .. } => {
                let what = match kind {
                    ChannelKind::Text => shown(name),
                    _ => format!("{} channel {}", kind.as_str(), name.trim()),
                };
                match category.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                    Some(c) => format!("create {what} in {c}"),
                    None => format!("create {what}"),
                }
            }
            Change::Rename { channel, name } => format!("rename {} to {}", channel.trim(), name.trim()),
            Change::Move { channel, category } => match category.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                Some(c) => format!("move {} into {c}", channel.trim()),
                None => format!("move {} out of its category", channel.trim()),
            },
            Change::Archive { channel } => format!("archive {}", channel.trim()),
        }
    }
}

/// The one-line subject of a Discord asking tool's call, for the user's yes (Claude's approval card).
pub fn ask_title(tool: &str, input: &Value) -> Option<String> {
    let field = |k: &str| input.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let server = field("connection").map(|c| format!("Discord ({c})")).unwrap_or_else(|| "Discord".into());
    match tool {
        "discord_change" => {
            let changes: Vec<Change> = serde_json::from_value(input.get("changes")?.clone()).ok()?;
            let all: Vec<String> = changes.iter().map(Change::describe).collect();
            Some(format!("{server}: {}", all.join("; ")))
        }
        "discord_webhook" => {
            let channel = shown(field("channel")?);
            let create = input.get("createChannel").and_then(Value::as_bool) == Some(true);
            let made = if create { " (made if missing)" } else { "" };
            Some(format!("{server}: a webhook on {channel}{made}, saved as connection {}", field("name")?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ch(id: &str, name: &str, kind: ChannelKind) -> Channel {
        Channel { id: id.into(), name: name.into(), kind, parent_id: None, position: 0, topic: None }
    }

    #[test]
    fn a_bot_connection_names_its_server() {
        assert_eq!(
            bot_config(&json!({"guildId": " 123 ", "pmActs": true})),
            Ok(BotConfig { guild_id: "123".into(), pm_acts: true, ..Default::default() })
        );
        // Talking to the PM in DMs needs the user's id (PDT-304); Inbox items as DMs only with it.
        let dms = bot_config(&json!({"guildId": "1", "dmUserId": " 77 ", "dmInbox": true})).unwrap();
        assert_eq!((dms.dm_user_id.as_deref(), dms.dm_inbox), (Some("77"), true));
        assert!(!bot_config(&json!({"guildId": "1", "dmUserId": "", "dmInbox": true})).unwrap().dm_inbox);
        assert!(bot_config(&json!({"guildId": "1", "dmUserId": "me"})).unwrap_err().contains("user id"));
        assert!(!bot_config(&json!({"guildId": "1"})).unwrap().pm_acts);
        assert!(bot_config(&json!({})).unwrap_err().contains("server's id"));
        assert!(bot_config(&json!({"guildId": "my server"})).unwrap_err().contains("all digits"));
    }

    #[test]
    fn channels_come_from_discord_objects() {
        let c = Channel::from_api(&json!({"id": "9", "name": "general", "type": 0, "parent_id": "4", "position": 2,
                                          "topic": ""}))
        .unwrap();
        assert_eq!((c.kind, c.parent_id.as_deref(), c.position, c.topic), (ChannelKind::Text, Some("4"), 2, None));
        assert_eq!(ChannelKind::from_type(4), ChannelKind::Category);
        assert_eq!(ChannelKind::from_type(13), ChannelKind::Other);
        assert_eq!(ChannelKind::Forum.to_type(), Some(15));
        assert!(Channel::from_api(&json!({"name": "x"})).is_none());
    }

    #[test]
    fn channel_names_are_made_the_way_discord_makes_them() {
        assert_eq!(valid_name(" #AI Briefs ", ChannelKind::Text).unwrap(), "ai-briefs");
        assert_eq!(valid_name("Bots and Briefs", ChannelKind::Category).unwrap(), "Bots and Briefs");
        assert!(valid_name("  ", ChannelKind::Text).is_err());
        assert!(valid_name(&"x".repeat(101), ChannelKind::Category).is_err());
    }

    #[test]
    fn channels_are_found_by_id_or_name() {
        let all = [
            ch("1", "Bots", ChannelKind::Category),
            ch("2", "ai-briefs", ChannelKind::Text),
            ch("3", "alerts", ChannelKind::Text),
            ch("4", "alerts", ChannelKind::Voice),
        ];
        assert_eq!(find(&all, "#AI Briefs", false).unwrap().id, "2");
        assert_eq!(find(&all, "2", false).unwrap().id, "2");
        assert_eq!(find(&all, "bots", true).unwrap().id, "1");
        assert!(find(&all, "Bots", false).unwrap_err().contains("no channel called Bots"));
        let e = find(&all, "alerts", false).unwrap_err();
        assert!(e.contains("2 channels") && e.contains("3, 4"), "{e}");
    }

    #[test]
    fn changes_read_as_a_line_for_the_users_yes() {
        let changes = json!([
            {"op": "create", "name": "ai-briefs", "category": "Bots"},
            {"op": "create", "name": "Bots", "kind": "category"},
            {"op": "create", "name": "Standup", "kind": "voice"},
            {"op": "rename", "channel": "#old", "name": "new"},
            {"op": "move", "channel": "#x"},
            {"op": "archive", "channel": "#y"},
        ]);
        let title = ask_title("discord_change", &json!({"connection": "home", "changes": changes})).unwrap();
        assert_eq!(
            title,
            "Discord (home): create #ai-briefs in Bots; create category Bots; create voice channel Standup; rename \
             #old to new; move #x out of its category; archive #y"
        );
        let title =
            ask_title("discord_webhook", &json!({"channel": "ai-briefs", "name": "briefs", "createChannel": true}));
        assert_eq!(title.unwrap(), "Discord: a webhook on #ai-briefs (made if missing), saved as connection briefs");
        assert_eq!(ask_title("discord_get", &json!({})), None);
        assert!(serde_json::from_value::<Change>(json!({"op": "delete", "channel": "x"})).is_err());
    }
}
