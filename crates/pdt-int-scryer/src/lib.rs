//! Scryer (films, series and anime, PDT-147): its connection and its actions (each a node kind and a tool), and its
//! whole GraphQL API for the PM (PDT-150), with the credentials an agent writes into a call kept out of what PDT
//! stores. pdtd's `scryer` feature.

mod actions;
mod core;
mod graphql;

use crate::core::SCRYER;
use pdt_api::sdk::{AnyAction, Integration};
use pdt_core::integration::ConnectionKind;
use serde_json::Value;
use std::sync::Arc;

pub use crate::actions::*;
pub use crate::graphql::{
    ScryerMutateConfig, ScryerMutateOutput, ScryerQueryConfig, ScryerQueryOutput, ScryerSchemaArgs,
};

/// What the PM is told about the Scryer tools.
const PM: &str = "- Media (Scryer): `scryer_library` browses the user's library (films, series, anime; search, monitored or not), \
`scryer_queue` shows what is downloading, and `scryer_search` looks titles up to add, each saying whether it is in the \
library already. `scryer_add` adds one by the ids the search gave (monitored unless told otherwise; Scryer then finds \
it); it waits for the user's yes unless the Scryer connection lets you act.\n\
- Scryer's settings (indexers, download clients, seeding profiles, quality, general settings, jobs): its whole \
GraphQL API. Find what you need with `scryer_schema` (`search` words, then an `operation` or `type`), read with \
`scryer_query` (a query document and variables; no yes needed), and change with `scryer_mutate` (one mutation, its \
arguments and an optional selection), which waits for the user's yes unless the connection lets you act; deleting, \
resetting and other high-impact changes always ask.";

pub struct Scryer;

impl Integration for Scryer {
    fn id(&self) -> &'static str {
        "scryer"
    }

    fn description(&self) -> &'static str {
        "Films, series and anime from a Scryer media manager: workflow actions, and the PM's tools for its library and \
         settings."
    }

    fn ask_title(&self, tool: &str, input: &Value) -> Option<String> {
        match tool {
            "scryer_add" => Some(core::ask_title(input).unwrap_or_else(|| "Scryer".into())),
            "scryer_mutate" => Some(core::mutate_ask_title(input).unwrap_or_else(|| "Scryer".into())),
            _ => None,
        }
    }

    fn redact_tool_input(&self, _tool: &str, input: &mut Value) -> Vec<String> {
        let secrets = core::secret_values(input);
        if !secrets.is_empty() {
            *input = core::redact(input);
        }
        secrets
    }

    fn pm_instructions(&self) -> Option<&'static str> {
        Some(PM)
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
}
