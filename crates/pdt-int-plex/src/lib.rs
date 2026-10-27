//! Plex: the `plex` connection and the Search, Recently added and Refresh actions. pdtd's `plex` feature.

mod nodes;

use crate::nodes::{PLEX, RecentlyAdded, Refresh, Search};
use pdt_api::sdk::{Integration, NodeKind};
use pdt_core::integration::ConnectionKind;

pub use crate::nodes::{PlexItem, PlexItems, PlexRefreshed, PlexSearchConfig, PlexSectionConfig};

pub struct Plex;

impl Integration for Plex {
    fn id(&self) -> &'static str {
        "plex"
    }

    fn description(&self) -> &'static str {
        "Search a Plex server, list what it added lately and refresh its libraries from workflows."
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: PLEX.into(),
            label: "Plex server".into(),
            plural: "Plex servers".into(),
            setup: "A Plex Media Server that Search, Recently added and Refresh steps use.".into(),
            description: "A Plex library: search, what’s new, refresh (the Plex actions).".into(),
            url: true,
            url_placeholder: Some("http://192.168.1.10:32400".into()),
            secret_label: "Plex token".into(),
            secret_hint: "In Plex Web: an item’s ⋯ → Get Info → View XML; the address that opens ends in \
                          X-Plex-Token=<token>."
                .into(),
            ..Default::default()
        }]
    }

    fn node_kinds(&self) -> Vec<Box<dyn NodeKind>> {
        vec![Box::new(Search), Box::new(RecentlyAdded), Box::new(Refresh)]
    }
}
