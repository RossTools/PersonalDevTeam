//! Scryer (films, series and anime): its connection and its actions (each a node kind and a tool).

use super::Integration;
use super::action::AnyAction;
use pdt_core::integration::ConnectionKind;
use pdt_core::scryer::SCRYER;
use std::sync::Arc;

pub struct Scryer;

impl Integration for Scryer {
    fn id(&self) -> &'static str {
        "scryer"
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
        crate::scryer::actions()
    }
}
