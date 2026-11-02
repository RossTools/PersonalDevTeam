//! Scryer (films, series and anime, PDT-147): its connection and its actions (each a node kind and a tool), and its
//! whole GraphQL API for the PM (PDT-150), with the credentials an agent writes into a call kept out of what PDT
//! stores. Its finished downloads are a workflow trigger (`scryer.download_finished`, PDT-365). pdtd's `scryer`
//! feature.

mod actions;
mod core;
mod graphql;
mod sensor;

use crate::core::SCRYER;
use pdt_api::sdk::{AnyAction, AppState, BoxFut, Integration, Poller};
use pdt_core::integration::ConnectionKind;
use pdt_core::workflow::events::EventType;
use serde_json::Value;
use std::sync::Arc;

pub use crate::actions::*;
pub use crate::graphql::{
    ScryerMutateConfig, ScryerMutateOutput, ScryerQueryConfig, ScryerQueryOutput, ScryerSchemaArgs,
};
pub use crate::sensor::{SCRYER_DOWNLOAD_FINISHED, ScryerDownloadFinished, refresh_scryer};

pub struct Scryer;

impl Integration for Scryer {
    fn id(&self) -> &'static str {
        "scryer"
    }

    fn approval_title(&self, tool: &str, input: &Value) -> Option<String> {
        let title = match tool {
            "scryer_add" => core::ask_title(input),
            "scryer_mutate" => core::mutate_ask_title(input),
            _ => return None,
        };
        Some(title.unwrap_or_else(|| "Scryer".into()))
    }

    /// A credential the agent wrote into a call (PDT-150): a password in a mutation's arguments, say.
    fn redact_call(&self, _tool: &str, input: &mut Value) -> Vec<String> {
        let secrets = core::secret_values(input);
        if !secrets.is_empty() {
            *input = core::redact(input);
        }
        secrets
    }

    fn pm_prompt(&self) -> Option<&'static str> {
        Some(PM_PROMPT)
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: SCRYER.into(),
            label: "Scryer".into(),
            plural: "Scryer".into(),
            setup: "A Scryer media manager (films, series and anime): the PM browses its library, looks titles up and \
                    adds them, and helps set Scryer up (indexers, download clients, settings) through its API; Search, \
                    Add, queue, library, Query and Change steps use it."
                .into(),
            description: "Films, series and anime: library, search, add, what’s downloading (the PM and the Scryer \
                          actions)."
                .into(),
            url: true,
            url_placeholder: Some("http://192.168.1.10:8080".into()),
            url_hint: Some("Scryer’s address, with its base path if it has one (http://nas/scryer).".into()),
            secret_label: "API key".into(),
            secret_hint: "In Scryer: Settings → API keys → Create. It starts with ska_.".into(),
            pm_acts: Some(
                "The PM adds titles and changes settings without asking (deleting, resetting and passing on a \
                 credential still ask)"
                    .into(),
            ),
            ..Default::default()
        }]
    }

    fn actions(&self) -> Vec<Arc<dyn AnyAction>> {
        actions::actions()
    }

    fn event_types(&self) -> &'static [EventType] {
        sensor::EVENT_TYPES
    }

    fn pollers(&self) -> Vec<Poller> {
        fn run(st: AppState) -> BoxFut<'static, ()> {
            Box::pin(async move { refresh_scryer(&st).await })
        }
        vec![Poller { name: "the Scryer check", run }]
    }
}

/// What the PM's prompt says about the Scryer tools.
pub(crate) const PM_PROMPT: &str = "- Media (Scryer): `scryer_library` browses the user's library (films, series, anime; search, \
monitored or not), `scryer_queue` shows what is downloading, and `scryer_search` looks titles up to add, each saying \
whether it is in the library already. `scryer_add` adds one by the ids the search gave (monitored unless told \
otherwise; Scryer then finds it); it waits for the user's yes unless the Scryer connection lets you act.\n\
- Scryer's settings (indexers, download clients, seeding profiles, quality, general settings, jobs): its whole GraphQL \
API. Find what you need with `scryer_schema` (`search` words, then an `operation` or `type`), read with `scryer_query` \
(a query document and variables; no yes needed), and change with `scryer_mutate` (one mutation, its arguments and an \
optional selection), which waits for the user's yes unless the connection lets you act; deleting, resetting and other \
high-impact changes always ask. Never ask the user to paste a password or API key into the chat: they save it as a PDT \
connection's secret (on its plugin's page in Settings → Plugins, e.g. an HTTP service), and you pass `{\"$secret\": \"<connection>\"}` \
where the value goes; PDT fills it in.";
