//! Talking to the PM through the bot's DMs (PDT-304). A `discord_bot` connection with the user's Discord user id
//! (`config.dmUserId`) holds a Gateway connection ([`crate::gateway`]); the user's DMs go to their current Discord
//! conversation (an ordinary PM conversation, marked `via` discord), and its replies, its calls that need a yes
//! (Approve and Decline buttons) and its questions (a button per option) come back as DMs. Whichever of Discord and
//! PDT's web answers a request first wins; the other's message is then edited to say so. Nobody else's DMs or presses
//! are taken, and server channels are ignored. Inbox items can be sent as DMs too (`config.dmInbox`).

use crate::bot::Bot;
use crate::chat::{self, Delivery, EPHEMERAL, Press, Question};
use crate::core::{self as core, DISCORD_BOT};
use crate::gateway::{self, Dispatch, Report};
use pdt_api::sdk::AppState;
use pdt_api::sdk::conversations::{answer_as_user, is_working, subscribe_all};
use pdt_core::conversation::{
    AgentEvent, Answer, ConversationEvent, MessageSource, RequestKind, RequestState, TurnPhase,
};
use pdt_core::integration::{ConnectionStatus, StatusTone};
use pdt_core::notification::{Change, Notification, NotificationKind};
use pdt_core::workflow::api::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

/// The `via` of a PM conversation held in the DMs: the plugin's id, which the web names it by.
pub(crate) const VIA: &str = "discord";
/// The host value that keeps the DMs' current conversation across restarts.
const STATE_KEY: &str = "discord.dm";
/// How often the wanted link is checked against the running one (a connection changed, the plugin turned off).
const CHECK_EVERY: Duration = Duration::from_secs(5);
/// How long a request waits before it is posted: one PDT answers for the user (a project that lets the PM act) never
/// reaches Discord.
const PROMPT_DELAY: Duration = Duration::from_millis(1500);
/// How often "typing…" is shown again while the PM works (Discord's lasts 10 s).
const TYPING_EVERY: Duration = Duration::from_secs(8);

/// The DM link's state, shown on its connection ([`connection_status`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DmStatus {
    /// The bot connection holding it; None when no connection has a user id.
    pub connection: Option<String>,
    /// `off`, `connecting`, `connected`, `retrying` or `failed`.
    pub state: String,
    /// Why it is retrying or failed.
    pub error: Option<String>,
    pub since: i64,
}

static STATUS: Mutex<Option<DmStatus>> = Mutex::new(None);

fn set_status(connection: Option<&str>, state: &str, error: Option<String>) {
    let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    let same =
        s.as_ref().is_some_and(|x| x.connection.as_deref() == connection && x.state == state && x.error == error);
    if !same {
        *s = Some(DmStatus {
            connection: connection.map(String::from),
            state: state.into(),
            error,
            since: pdt_api::sdk::now_ms(),
        });
    }
}

/// The DM link's state now.
pub(crate) fn status() -> DmStatus {
    STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| DmStatus { state: "off".into(), ..Default::default() })
}

/// A bot connection's DMs with the PM, on its row (`GET /api/connections/{id}/status`): whether its Gateway
/// connection is up, or why not. None for a bot without the user's id (it has no DMs).
pub(crate) fn connection_status(c: &Connection) -> Option<ConnectionStatus> {
    let has_user = c.config.get("dmUserId").and_then(Value::as_str).is_some_and(|u| !u.trim().is_empty());
    if c.kind != DISCORD_BOT || !has_user {
        return None;
    }
    let s = status();
    let mine = s.connection.as_deref() == Some(c.name.as_str());
    let (state, error) = if mine { (s.state.as_str(), s.error.clone()) } else { ("off", None) };
    let mut text = match state {
        "connected" => "Connected",
        "connecting" => "Connecting…",
        "retrying" => "Reconnecting",
        "failed" => "Not connected",
        "off" => "Off",
        other => other,
    }
    .to_string();
    if let Some(other) = s.connection.as_deref().filter(|_| !mine) {
        text = format!("{text} (another bot, {other}, holds them)");
    }
    let tone = match state {
        "connected" => StatusTone::Ok,
        "failed" => StatusTone::Failed,
        _ => StatusTone::Neutral,
    };
    Some(ConnectionStatus { label: "DMs with the PM".into(), state: text, tone, error })
}

/// What a link is made from: when any of it changes, the link starts over.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Want {
    connection: String,
    token: String,
    user: String,
    inbox: bool,
    url: String,
    updated_at: i64,
}

/// The link the settings ask for: the first enabled bot connection with a user id and a token.
async fn wanted(st: &AppState) -> Option<Want> {
    if !st.integrations.is_enabled("discord") {
        return None;
    }
    let all = st.store.list_connections().await.ok()?;
    let conn = all
        .into_iter()
        .find(|c| c.kind == DISCORD_BOT && core::bot_config(&c.config).is_ok_and(|b| b.dm_user_id.is_some()))?;
    let cfg = core::bot_config(&conn.config).ok()?;
    let token = match pdt_api::sdk::connections::connection_with_secret(st, &conn.name).await {
        Ok((_, Some(t))) if !t.trim().is_empty() => t.trim().to_string(),
        _ => {
            set_status(Some(&conn.name), "failed", Some(format!("{} has no bot token", conn.name)));
            return None;
        }
    };
    Some(Want {
        connection: conn.name,
        token,
        user: cfg.dm_user_id?,
        inbox: cfg.dm_inbox,
        url: conn.url,
        updated_at: conn.updated_at,
    })
}

/// The service: keeps the wanted link running, and none while nothing asks for one.
pub(crate) fn service(st: AppState) -> pdt_api::sdk::BoxFut<'static, ()> {
    Box::pin(async move {
        let mut running: Option<(Want, tokio::task::JoinHandle<()>)> = None;
        loop {
            let want = wanted(&st).await;
            if running.as_ref().map(|(w, _)| w) != want.as_ref() {
                if let Some((w, task)) = running.take() {
                    tracing::info!(connection = %w.connection, "stopping the Discord bot's DM link");
                    task.abort();
                }
                match want {
                    Some(w) => {
                        tracing::info!(connection = %w.connection, "starting the Discord bot's DM link");
                        set_status(Some(&w.connection), "connecting", None);
                        let task = tokio::spawn(link(st.clone(), w.clone()));
                        running = Some((w, task));
                    }
                    None => {
                        let failed = status().state == "failed" && status().connection.is_some();
                        if !failed || !st.integrations.is_enabled("discord") {
                            set_status(None, "off", None);
                        }
                    }
                }
            }
            if st.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                if let Some((_, task)) = running.take() {
                    task.abort();
                }
                return;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    })
}

/// One link: the Gateway connection and the bridge between it and PDT, until it fails for good or is aborted.
async fn link(st: AppState, want: Want) {
    let name = want.connection.clone();
    let mut tries = 0u32;
    let (bot, url) = loop {
        let opened = match Bot::open(&st, Some(&name)).await {
            Ok((_, bot)) => bot.gateway_url().await.map(|u| (bot, u)),
            Err(e) => Err(e),
        };
        match opened {
            Ok(x) => break x,
            Err(e) if e.contains("401") => {
                // The connection's error first: whoever sees the status sees it too.
                pdt_api::sdk::connections::report(&st, &name, Some(&e)).await;
                set_status(Some(&name), "failed", Some(e));
                return;
            }
            Err(e) => {
                tries += 1;
                set_status(Some(&name), "retrying", Some(e));
                tokio::time::sleep(gateway::backoff(tries)).await;
            }
        }
    };
    let (tx, mut rx) = mpsc::channel(64);
    let gw = tokio::spawn(gateway::run(url, want.token.clone(), tx));
    let _stop = AbortOnDrop(gw);
    let mut bridge = Bridge::new(st.clone(), bot, &want).await;
    let mut events = subscribe_all(&st);
    let mut notes = st.notifier.subscribe();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                // The Gateway task ended without saying why (it panicked): not connected any more.
                None => {
                    let why = "the Discord Gateway connection stopped unexpectedly".to_string();
                    pdt_api::sdk::connections::report(&st, &name, Some(&why)).await;
                    set_status(Some(&name), "failed", Some(why));
                    return;
                }
                Some(Report::Connected) => {
                    pdt_api::sdk::connections::report(&st, &name, None).await;
                    set_status(Some(&name), "connected", None);
                }
                Some(Report::Retrying(why)) => set_status(Some(&name), "retrying", Some(why)),
                Some(Report::Failed(why)) => {
                    pdt_api::sdk::connections::report(&st, &name, Some(&why)).await;
                    set_status(Some(&name), "failed", Some(why));
                    return;
                }
                Some(Report::Event(d)) => bridge.dispatch(d).await,
            },
            e = events.recv() => match e {
                Ok((session, event)) => bridge.conversation(&session, event).await,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "the Discord DM link fell behind the conversations' events");
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            n = notes.recv() => {
                if let Ok((Change::Created, n)) = n {
                    bridge.notification(&n).await;
                }
            }
            _ = tick.tick() => bridge.tick().await,
        }
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What the DMs keep across restarts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    /// The conversation the DMs talk to.
    #[serde(default)]
    current: Option<String>,
    /// The title the last message was headed by (a reply from another conversation names its own).
    #[serde(default)]
    last_title: Option<String>,
}

/// A turn's text so far, in a conversation held in the DMs.
#[derive(Debug, Default)]
struct Turn {
    /// The user's message started it (not one of PDT's own: an update, a journal request).
    user: bool,
    /// The PM's messages (item id, text), in order.
    texts: Vec<(String, String)>,
}

/// A request posted (or about to be) in the DMs.
#[derive(Debug)]
struct Prompt {
    key: String,
    session: String,
    request: String,
    due: Instant,
    what: What,
    /// The messages it was posted as: id and text (one per question).
    posted: Vec<(String, String)>,
}

#[derive(Debug)]
enum What {
    Approval { title: String, tool: String, input: Value, plan: bool },
    Questions { all: Vec<Question>, picked: Vec<Vec<String>>, answers: Vec<Option<Vec<String>>> },
}

impl What {
    fn approve(&self, yes: bool) -> Answer {
        match self {
            What::Approval { plan: true, .. } => Answer::Plan { approve: yes, feedback: None, mode: None },
            _ if yes => Answer::Approve { always: false },
            _ => Answer::Deny { message: None },
        }
    }
}

struct Bridge {
    st: AppState,
    bot: Bot,
    user: String,
    inbox: bool,
    /// The DM channel with the user.
    dm: Option<String>,
    saved: Saved,
    /// The conversations held in the DMs.
    via: HashSet<String>,
    turns: HashMap<String, Turn>,
    prompts: Vec<Prompt>,
    next_key: u64,
    last_typing: Option<Instant>,
}

impl Bridge {
    async fn new(st: AppState, bot: Bot, want: &Want) -> Self {
        let saved = match st.store.get_host_value(STATE_KEY).await {
            Ok(Some(v)) => serde_json::from_value(v).unwrap_or_default(),
            _ => Saved::default(),
        };
        let via = pdt_api::sdk::pm::conversations(&st)
            .await
            .map(|all| all.into_iter().filter(|c| c.via.as_deref() == Some(VIA)).map(|c| c.session.id).collect())
            .unwrap_or_default();
        Bridge {
            st,
            bot,
            user: want.user.clone(),
            inbox: want.inbox,
            dm: None,
            saved,
            via,
            turns: HashMap::new(),
            prompts: vec![],
            next_key: 1,
            last_typing: None,
        }
    }

    async fn save(&self) {
        let v = serde_json::to_value(&self.saved).unwrap_or(Value::Null);
        if let Err(e) = self.st.store.put_host_value(STATE_KEY, &v).await {
            tracing::warn!(error = %e, "could not save the Discord DMs' state");
        }
    }

    /// The DM channel, opened when PDT doesn't know it yet.
    async fn dm(&mut self) -> Option<String> {
        if self.dm.is_none() {
            match self.bot.open_dm(&self.user).await {
                Ok(c) => self.dm = Some(c),
                Err(e) => tracing::warn!(error = %e, "could not open the Discord DM channel"),
            }
        }
        self.dm.clone()
    }

    /// Posts a message in the DMs; its id.
    async fn post(&mut self, body: Value, file: Option<(&str, &str)>) -> Option<String> {
        let dm = self.dm().await?;
        match self.bot.send(&dm, body, file).await {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(error = %e, "could not send a Discord DM");
                None
            }
        }
    }

    async fn say(&mut self, text: &str) {
        for piece in chat::split_markdown(text, chat::MAX_MESSAGE) {
            self.post(json!({"content": piece, "allowed_mentions": {"parse": []}}), None).await;
        }
    }

    /// The conversation the DMs talk to, while it is open.
    async fn current(&mut self) -> Option<(String, String)> {
        let id = self.saved.current.clone()?;
        let open = self.open_conversations().await;
        match open.into_iter().find(|(i, _)| *i == id) {
            Some(c) => Some(c),
            None => {
                self.saved.current = None;
                self.save().await;
                None
            }
        }
    }

    /// The open PM conversations (id, title), the most recent first.
    async fn open_conversations(&self) -> Vec<(String, String)> {
        pdt_api::sdk::pm::conversations(&self.st)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.open)
            .map(|c| (c.session.id, c.title))
            .collect()
    }

    /// The title a message from `session` is headed by: its own, when the last message was another's.
    async fn head(&mut self, session: &str) -> Option<String> {
        let title = self.st.store.get_session(session).await.ok().flatten().map(|s| s.title)?;
        if self.saved.last_title.as_deref() == Some(title.as_str()) {
            return None;
        }
        self.saved.last_title = Some(title.clone());
        self.save().await;
        Some(title)
    }

    async fn make_current(&mut self, id: &str, title: &str) {
        if self.via.insert(id.to_string())
            && let Err(e) = pdt_api::sdk::pm::set_via(&self.st, id, Some(VIA)).await
        {
            tracing::warn!(session = %id, error = %e.message(), "could not mark a PM conversation as held in Discord");
        }
        self.saved.current = Some(id.to_string());
        self.saved.last_title = Some(title.to_string());
        self.save().await;
    }

    // ---- the Gateway's events ----

    async fn dispatch(&mut self, d: Dispatch) {
        match d.name.as_str() {
            "READY" => {
                if let Some(app) = d.data["application"]["id"].as_str()
                    && let Err(e) = self.bot.set_commands(app, chat::commands()).await
                {
                    tracing::warn!(error = %e, "could not register the Discord bot's slash commands");
                }
            }
            "MESSAGE_CREATE" => self.message(&d.data).await,
            "INTERACTION_CREATE" => self.interaction(&d.data).await,
            _ => {}
        }
    }

    async fn message(&mut self, m: &Value) {
        // Server channels are not for the PM; neither are bots (its own messages among them).
        if m.get("guild_id").is_some_and(|g| !g.is_null()) || m["author"]["bot"] == true {
            return;
        }
        let author = m["author"]["id"].as_str().unwrap_or_default();
        if author != self.user {
            tracing::info!(author, "ignored a Discord DM from someone the bot is not set up for");
            return;
        }
        if let Some(c) = m["channel_id"].as_str() {
            self.dm = Some(c.to_string());
        }
        let text = m["content"].as_str().unwrap_or_default().trim().to_string();
        if text.is_empty() {
            self.say("Only text reaches the PM for now.").await;
            return;
        }
        if self.typed_answer(&text).await {
            return;
        }
        self.pass_on(&text).await;
    }

    /// Sends the user's message to the current conversation, or a new one when none is open.
    async fn pass_on(&mut self, text: &str) {
        match self.current().await {
            Some((id, _)) => {
                if let Err(e) = pdt_api::sdk::pm::send_from_user(&self.st, &id, text).await {
                    self.say(&format!("PDT could not pass that on: {}", e.message())).await;
                    return;
                }
            }
            None => match pdt_api::sdk::pm::start_conversation(&self.st, None, Some(text.to_string()), VIA).await {
                Ok(s) => {
                    self.via.insert(s.id.clone());
                    self.saved.current = Some(s.id.clone());
                    self.save().await;
                }
                Err(e) => {
                    self.say(&format!("PDT could not start a PM conversation: {}", e.message())).await;
                    return;
                }
            },
        }
        self.typing(true).await;
    }

    async fn interaction(&mut self, i: &Value) {
        let (Some(id), Some(token)) = (i["id"].as_str(), i["token"].as_str()) else { return };
        let (id, token) = (id.to_string(), token.to_string());
        let user = i["user"]["id"].as_str().or_else(|| i["member"]["user"]["id"].as_str()).unwrap_or_default();
        if user != self.user {
            tracing::info!(user, "refused a Discord interaction from someone the bot is not set up for");
            let body = json!({"type": 4, "data": {"content": "This bot only answers the user it is set up for.",
                                                  "flags": EPHEMERAL}});
            self.reply(&id, &token, body).await;
            return;
        }
        if i.get("guild_id").is_none_or(Value::is_null)
            && let Some(c) = i["channel_id"].as_str()
        {
            self.dm = Some(c.to_string());
        }
        let body = match i["type"].as_u64() {
            Some(2) => self.command(&i["data"]).await,
            Some(3) => self.press(i).await,
            Some(4) => {
                let typed = i["data"]["options"][0]["value"].as_str().unwrap_or_default().to_string();
                let open = self.open_conversations().await;
                let choices = chat::choices(&typed, open.iter().map(|(id, t)| (id.as_str(), t.as_str())));
                json!({"type": 8, "data": {"choices": choices}})
            }
            _ => return,
        };
        self.reply(&id, &token, body).await;
    }

    async fn reply(&self, id: &str, token: &str, body: Value) {
        if let Err(e) = self.bot.reply(id, token, body).await {
            tracing::warn!(error = %e, "could not answer a Discord interaction");
        }
    }

    /// A slash command's answer.
    async fn command(&mut self, data: &Value) -> Value {
        let option = |name: &str| {
            data["options"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|o| o["name"] == name)
                .and_then(|o| o["value"].as_str())
        };
        let text = match data["name"].as_str().unwrap_or_default() {
            "new" => {
                let topic = option("topic").map(String::from);
                match pdt_api::sdk::pm::start_conversation(&self.st, topic.as_deref(), None, VIA).await {
                    Ok(s) => {
                        self.make_current(&s.id, &s.title).await;
                        format!("Started **{}**. What you write here goes to it.", s.title)
                    }
                    Err(e) => format!("PDT could not start a PM conversation: {}", e.message()),
                }
            }
            "list" => {
                let current = self.current().await.map(|(id, _)| id);
                let all = pdt_api::sdk::pm::conversations(&self.st).await.unwrap_or_default();
                let open: Vec<String> = all
                    .iter()
                    .filter(|c| c.open)
                    .map(|c| {
                        let here = if current.as_deref() == Some(c.session.id.as_str()) { " (current)" } else { "" };
                        let via = if c.via.as_deref() == Some(VIA) { " · via Discord" } else { "" };
                        format!("- **{}**{here}{via} · <t:{}:R>", c.title, c.last_activity / 1000)
                    })
                    .collect();
                if open.is_empty() {
                    "No PM conversation is open. Write here to start one, or use /new.".into()
                } else {
                    format!("Open PM conversations:\n{}\n-# /switch talks to another.", open.join("\n"))
                }
            }
            "switch" => {
                let given = option("conversation").unwrap_or_default().to_string();
                let open = self.open_conversations().await;
                let pairs: Vec<(&str, &str)> = open.iter().map(|(i, t)| (i.as_str(), t.as_str())).collect();
                match chat::find_conversation(&given, &pairs) {
                    Ok(id) => {
                        let (id, title) = open.iter().find(|(i, _)| i == id).cloned().unwrap_or_default();
                        self.make_current(&id, &title).await;
                        format!("Now talking to **{title}**.")
                    }
                    Err(e) => e,
                }
            }
            "close" => match self.current().await {
                Some((id, title)) => match pdt_api::sdk::pm::close_conversation(&self.st, &id).await {
                    Ok(()) => {
                        self.saved.current = None;
                        self.save().await;
                        format!(
                            "Closing **{title}**: the PM is writing its journal entry. Your next message starts a new \
                             conversation."
                        )
                    }
                    Err(e) => format!("PDT could not close it: {}", e.message()),
                },
                None => "No conversation is open here. Write to start one.".into(),
            },
            other => format!("PDT has no /{other} command."),
        };
        json!({"type": 4, "data": {"content": text, "allowed_mentions": {"parse": []}}})
    }

    /// A button press's answer: the message it was on, updated.
    async fn press(&mut self, i: &Value) -> Value {
        let gone = |text: &str| json!({"type": 7, "data": {"content": text, "components": []}});
        let original = i["message"]["content"].as_str().unwrap_or_default().to_string();
        let Some((key, press)) = i["data"]["custom_id"].as_str().and_then(chat::parse_button) else {
            return gone(&original);
        };
        let Some(at) = self.prompts.iter().position(|p| p.key == key) else {
            return gone(&format!("{original}{}", chat::answered_line("No longer open", None)));
        };
        match press {
            Press::Approve | Press::Decline => {
                let yes = press == Press::Approve;
                let p = &self.prompts[at];
                let answer = p.what.approve(yes);
                let said = if yes { "Approved" } else { "Declined" };
                let (session, request) = (p.session.clone(), p.request.clone());
                self.prompts.remove(at);
                match answer_as_user(&self.st, &session, &request, &answer).await {
                    Ok(()) => {
                        json!({"type": 7, "data": {"content": format!("{original}{}", chat::answered_line(said, None)),
                                                   "components": chat::approval_buttons(&key, true)}})
                    }
                    Err(e) => {
                        tracing::info!(error = %e.message(), "a Discord answer came after its request closed");
                        gone(&format!("{original}{}", chat::answered_line("No longer open", None)))
                    }
                }
            }
            Press::Pick(qi, oi) => {
                let p = &mut self.prompts[at];
                let What::Questions { all, picked, answers } = &mut p.what else { return gone(&original) };
                let (Some(q), Some(opt)) = (all.get(qi).cloned(), all.get(qi).and_then(|q| q.options.get(oi)).cloned())
                else {
                    return gone(&original);
                };
                if q.multi {
                    let mine = &mut picked[qi];
                    if let Some(at) = mine.iter().position(|x| *x == opt) {
                        mine.remove(at);
                    } else {
                        mine.push(opt);
                    }
                    let (text, rows) = chat::question_message(&key, qi, &q, mine, false);
                    return json!({"type": 7, "data": {"content": text, "components": rows}});
                }
                answers[qi] = Some(vec![opt.clone()]);
                let body = answered_question(&key, qi, &q, &[opt]);
                self.questions_done(at).await;
                body
            }
            Press::Done(qi) => {
                let p = &mut self.prompts[at];
                let What::Questions { all, picked, answers } = &mut p.what else { return gone(&original) };
                let Some(q) = all.get(qi).cloned() else { return gone(&original) };
                let chosen = picked[qi].clone();
                answers[qi] = Some(chosen.clone());
                let body = answered_question(&key, qi, &q, &chosen);
                self.questions_done(at).await;
                body
            }
        }
    }

    /// A question prompt whose questions are all answered sends the answers.
    async fn questions_done(&mut self, at: usize) {
        let p = &self.prompts[at];
        let What::Questions { all, answers, .. } = &p.what else { return };
        if answers.iter().any(Option::is_none) {
            return;
        }
        let map: BTreeMap<String, Vec<String>> =
            all.iter().zip(answers).map(|(q, a)| (q.question.clone(), a.clone().unwrap_or_default())).collect();
        let (session, request) = (p.session.clone(), p.request.clone());
        self.prompts.remove(at);
        if let Err(e) = answer_as_user(&self.st, &session, &request, &Answer::Question { answers: map }).await {
            self.say(&format!("The question closed before the answer reached it: {}", e.message())).await;
        }
    }

    /// A DM while a question waits answers its first open question; true when it did.
    async fn typed_answer(&mut self, text: &str) -> bool {
        let Some(at) =
            self.prompts.iter().position(|p| matches!(p.what, What::Questions { .. }) && !p.posted.is_empty())
        else {
            return false;
        };
        let (key, posted) = (self.prompts[at].key.clone(), self.prompts[at].posted.clone());
        let What::Questions { all, answers, .. } = &mut self.prompts[at].what else { return false };
        let Some(qi) = answers.iter().position(Option::is_none) else { return false };
        answers[qi] = Some(vec![text.to_string()]);
        let q = all[qi].clone();
        if let (Some(dm), Some((msg, _))) = (self.dm.clone(), posted.get(qi)) {
            let body = answered_question(&key, qi, &q, &[text.to_string()]);
            if let Err(e) = self.bot.edit(&dm, msg, body["data"].clone()).await {
                tracing::warn!(error = %e, "could not update a Discord question");
            }
        }
        self.questions_done(at).await;
        true
    }

    // ---- PDT's side ----

    async fn conversation(&mut self, session: &str, row: ConversationEvent) {
        if !self.via.contains(session) {
            return;
        }
        match row.event {
            AgentEvent::UserMessage { source, .. } => {
                self.turns.entry(session.to_string()).or_default().user = source != MessageSource::Notice;
            }
            AgentEvent::AgentMessage { id, markdown, .. } => {
                let turn = self.turns.entry(session.to_string()).or_default();
                if !turn.user {
                    return;
                }
                let item = row.item_id.unwrap_or(id);
                match turn.texts.iter_mut().find(|(i, _)| *i == item) {
                    Some(t) => t.1 = markdown,
                    None => turn.texts.push((item, markdown)),
                }
            }
            AgentEvent::Turn { phase: TurnPhase::Ended, .. } => {
                self.flush(session).await;
                self.turns.remove(session);
            }
            AgentEvent::Request { id, request_kind, tool, title, input, state, answer, .. } => match state {
                RequestState::Open => {
                    if self.prompts.iter().any(|p| p.session == session && p.request == id) {
                        return;
                    }
                    // What it said before asking comes first.
                    self.flush(session).await;
                    let what = match request_kind {
                        RequestKind::Question => {
                            let all = chat::questions(&input);
                            if all.is_empty() {
                                return;
                            }
                            let n = all.len();
                            What::Questions { all, picked: vec![vec![]; n], answers: vec![None; n] }
                        }
                        RequestKind::Approval | RequestKind::Plan => {
                            What::Approval { title, tool, input, plan: request_kind == RequestKind::Plan }
                        }
                    };
                    let key = format!("{:x}", self.next_key);
                    self.next_key += 1;
                    self.prompts.push(Prompt {
                        key,
                        session: session.to_string(),
                        request: id,
                        due: Instant::now() + PROMPT_DELAY,
                        what,
                        posted: vec![],
                    });
                }
                RequestState::Answered | RequestState::Expired => {
                    let Some(at) = self.prompts.iter().position(|p| p.session == session && p.request == id) else {
                        return;
                    };
                    let p = self.prompts.remove(at);
                    let said = match (&state, &answer) {
                        (RequestState::Expired, _) => "No longer open",
                        (_, Some(Answer::Approve { .. })) | (_, Some(Answer::Plan { approve: true, .. })) => "Approved",
                        (_, Some(Answer::Question { .. })) => "Answered",
                        _ => "Declined",
                    };
                    let by = (state == RequestState::Answered).then_some("web");
                    self.settled(&p, said, by).await;
                }
            },
            _ => {}
        }
    }

    /// Edits a posted prompt answered elsewhere: what was answered, and its buttons off.
    async fn settled(&mut self, p: &Prompt, said: &str, by: Option<&str>) {
        let Some(dm) = self.dm.clone() else { return };
        for (i, (msg, text)) in p.posted.iter().enumerate() {
            let rows = match &p.what {
                What::Approval { .. } => chat::approval_buttons(&p.key, true),
                What::Questions { all, picked, .. } => chat::question_message(&p.key, i, &all[i], &picked[i], true).1,
            };
            let body = json!({"content": format!("{text}{}", chat::answered_line(said, by)), "components": rows});
            if let Err(e) = self.bot.edit(&dm, msg, body).await {
                tracing::warn!(error = %e, "could not update a Discord prompt");
            }
        }
    }

    /// Sends what the PM said in a user's turn so far.
    async fn flush(&mut self, session: &str) {
        let Some(turn) = self.turns.get_mut(session) else { return };
        if turn.texts.is_empty() {
            return;
        }
        let text: Vec<String> = turn.texts.drain(..).map(|(_, t)| t).filter(|t| !t.trim().is_empty()).collect();
        if text.is_empty() {
            return;
        }
        let head = self.head(session).await;
        match chat::deliver(head.as_deref(), &text.join("\n\n")) {
            Delivery::Messages(pieces) => {
                for piece in pieces {
                    self.post(json!({"content": piece, "allowed_mentions": {"parse": []}}), None).await;
                }
            }
            Delivery::File { note, name, body } => {
                let msg = json!({"content": note, "allowed_mentions": {"parse": []}});
                self.post(msg, Some((&name, &body))).await;
            }
        }
    }

    async fn tick(&mut self) {
        let now = Instant::now();
        let due: Vec<usize> = self
            .prompts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.posted.is_empty() && p.due <= now)
            .map(|(i, _)| i)
            .collect();
        for i in due {
            self.post_prompt(i).await;
        }
        self.typing(false).await;
    }

    async fn post_prompt(&mut self, at: usize) {
        let session = self.prompts[at].session.clone();
        let head = self.head(&session).await;
        let headed = |text: String| match &head {
            Some(h) => format!("**{h}**\n{text}"),
            None => text,
        };
        let key = self.prompts[at].key.clone();
        let mut posted = vec![];
        match &self.prompts[at].what {
            What::Approval { title, tool, input, .. } => {
                let (text, file) = chat::approval_text(title, tool, input);
                let text = headed(text);
                let body = json!({"content": text, "components": chat::approval_buttons(&key, false),
                                  "allowed_mentions": {"parse": []}});
                let file = file.as_deref().map(|f| ("arguments.json", f));
                if let Some(id) = self.post(body, file).await {
                    posted.push((id, text));
                }
            }
            What::Questions { all, picked, .. } => {
                let msgs: Vec<(String, Value)> = all
                    .iter()
                    .enumerate()
                    .map(|(qi, q)| {
                        let (text, rows) = chat::question_message(&key, qi, q, &picked[qi], false);
                        (if qi == 0 { headed(text) } else { text }, rows)
                    })
                    .collect();
                for (text, rows) in msgs {
                    let body = json!({"content": text, "components": rows, "allowed_mentions": {"parse": []}});
                    if let Some(id) = self.post(body, None).await {
                        posted.push((id, text));
                    }
                }
            }
        }
        let p = &mut self.prompts[at];
        if posted.is_empty() {
            // Discord refused it: try again later rather than every tick.
            p.due = Instant::now() + Duration::from_secs(30);
        }
        p.posted = posted;
    }

    /// Shows "typing…" while a conversation held in the DMs works (`now`: at once, else every [`TYPING_EVERY`]).
    async fn typing(&mut self, now: bool) {
        let due = now || self.last_typing.is_none_or(|t| t.elapsed() >= TYPING_EVERY);
        if !due {
            return;
        }
        let working = now || self.via.iter().any(|id| is_working(&self.st, id));
        if !working {
            return;
        }
        self.last_typing = Some(Instant::now());
        if let Some(dm) = self.dm().await
            && let Err(e) = self.bot.typing(&dm).await
        {
            tracing::debug!(error = %e, "could not show typing in Discord");
        }
    }

    async fn notification(&mut self, n: &Notification) {
        if !self.inbox || !n.kind.inbox_owned() {
            return;
        }
        // A conversation held here asks with its own buttons.
        let asks = matches!(n.kind, NotificationKind::InboxApproval | NotificationKind::InboxQuestion);
        if asks && n.session_id.as_deref().is_some_and(|s| self.via.contains(s)) {
            return;
        }
        let body = if n.body.trim().is_empty() { String::new() } else { format!("\n{}", n.body.trim()) };
        self.say(&format!("**{}**{body}", n.title.trim())).await;
    }
}

/// A question's message once answered: what was picked, its buttons off.
fn answered_question(key: &str, qi: usize, q: &Question, chosen: &[String]) -> Value {
    let (text, rows) = chat::question_message(key, qi, q, chosen, true);
    let said = if chosen.is_empty() { "(no answer)".to_string() } else { chosen.join(", ") };
    json!({"type": 7, "data": {"content": format!("{text}{}", chat::answered_line(&said, None)), "components": rows}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bot(name: &str, config: Value) -> Connection {
        Connection {
            id: format!("c-{name}"),
            name: name.into(),
            kind: DISCORD_BOT.into(),
            url: String::new(),
            config,
            has_secret: true,
            created_at: 0,
            updated_at: 0,
            last_error: None,
            last_error_at: None,
        }
    }

    /// One test, as the link's state is the process's.
    #[test]
    fn a_bot_connection_shows_whether_its_dms_are_connected() {
        let home = bot("home", json!({"guildId": "1", "dmUserId": "77"}));
        // Nothing has started a link.
        *STATUS.lock().unwrap() = None;
        assert_eq!(status().state, "off");
        let s = connection_status(&home).unwrap();
        assert_eq!((s.label.as_str(), s.state.as_str(), s.tone), ("DMs with the PM", "Off", StatusTone::Neutral));

        // A bot with no user, a blank one, or another kind of connection has no DMs.
        assert_eq!(connection_status(&bot("x", json!({"guildId": "1"}))), None);
        assert_eq!(connection_status(&bot("x", json!({"guildId": "1", "dmUserId": "  "}))), None);
        let mut hook = bot("hook", json!({"dmUserId": "77"}));
        hook.kind = "discord_webhook".into();
        assert_eq!(connection_status(&hook), None);

        set_status(Some("home"), "connecting", None);
        assert_eq!(connection_status(&home).unwrap().state, "Connecting…");
        set_status(Some("home"), "connected", None);
        let s = connection_status(&home).unwrap();
        assert_eq!((s.state.as_str(), s.tone, s.error), ("Connected", StatusTone::Ok, None));

        // The same state again keeps when it started; a new one restarts it.
        let since = status().since;
        std::thread::sleep(Duration::from_millis(5));
        set_status(Some("home"), "connected", None);
        assert_eq!(status().since, since);
        set_status(Some("home"), "retrying", Some("Gateway closed".into()));
        let s = connection_status(&home).unwrap();
        assert_eq!(
            (s.state.as_str(), s.tone, s.error.as_deref()),
            ("Reconnecting", StatusTone::Neutral, Some("Gateway closed"))
        );
        set_status(Some("home"), "failed", Some("home has no bot token".into()));
        let s = connection_status(&home).unwrap();
        assert_eq!((s.state.as_str(), s.tone), ("Not connected", StatusTone::Failed));

        // Another bot's row says which one holds the DMs, and none of its error.
        let other = bot("work", json!({"guildId": "2", "dmUserId": "77"}));
        let s = connection_status(&other).unwrap();
        assert_eq!(s.state, "Off (another bot, home, holds them)");
        assert_eq!((s.tone, s.error), (StatusTone::Neutral, None));
        *STATUS.lock().unwrap() = None;
    }

    #[test]
    fn approve_and_decline_answer_as_the_request_asks() {
        let call = What::Approval { title: "t".into(), tool: "Bash".into(), input: json!({}), plan: false };
        assert_eq!(call.approve(true), Answer::Approve { always: false });
        assert_eq!(call.approve(false), Answer::Deny { message: None });
        let plan = What::Approval { title: "t".into(), tool: "ExitPlanMode".into(), input: json!({}), plan: true };
        assert_eq!(plan.approve(true), Answer::Plan { approve: true, feedback: None, mode: None });
        assert_eq!(plan.approve(false), Answer::Plan { approve: false, feedback: None, mode: None });
        let ask = What::Questions { all: vec![], picked: vec![], answers: vec![] };
        assert_eq!(ask.approve(false), Answer::Deny { message: None });
    }

    #[test]
    fn what_the_dms_keep_reads_back_and_tolerates_missing_fields() {
        let saved = Saved { current: Some("s1".into()), last_title: Some("Plans".into()) };
        let v = serde_json::to_value(&saved).unwrap();
        assert_eq!(v, json!({"current": "s1", "lastTitle": "Plans"}));
        let back: Saved = serde_json::from_value(v).unwrap();
        assert_eq!((back.current.as_deref(), back.last_title.as_deref()), (Some("s1"), Some("Plans")));
        let empty: Saved = serde_json::from_value(json!({})).unwrap();
        assert_eq!((empty.current, empty.last_title), (None, None));
    }

    #[test]
    fn an_answered_question_shows_the_answer_with_its_buttons_off() {
        let q = Question {
            question: "Which one?".into(),
            header: "Pick".into(),
            options: vec!["A".into(), "B".into()],
            multi: true,
        };
        let v = answered_question("k1", 0, &q, &["A".into(), "B".into()]);
        assert_eq!(v["type"], 7, "an update of the pressed message");
        let content = v["data"]["content"].as_str().unwrap();
        assert!(content.starts_with("**Pick**\nWhich one?"), "{content}");
        assert!(content.ends_with("\n**A, B**"), "{content}");
        let rows = v["data"]["components"].as_array().unwrap();
        let buttons: Vec<&Value> = rows.iter().flat_map(|r| r["components"].as_array().unwrap()).collect();
        assert_eq!(buttons.len(), 3, "two options and Send");
        assert!(buttons.iter().all(|b| b["disabled"] == true), "{v}");
        assert!(buttons[..2].iter().all(|b| b["style"] == 3), "both picked: {v}");

        let none = answered_question("k1", 0, &q, &[]);
        assert!(none["data"]["content"].as_str().unwrap().ends_with("\n**(no answer)**"), "{none}");
    }
}
