//! Finished downloads (PDT-365): while an enabled workflow has a `scryer.download_finished` trigger, pdtd reads every
//! minute the newest successful downloads in each Scryer connection's history and logs `scryer.download_finished` for
//! each one it has not seen. The first look (or one after a long gap: the trigger was off, pdtd was down) only records
//! what is there (`sensor.scryer.<connection>`), so turning a trigger on never replays the history. A download whose
//! event could not be logged is left out of the record, so the next look tries it again. A failing connection is
//! reported once ([`pdt_api::sdk::connections`]).

use crate::actions::Client;
use crate::core::SCRYER;
use pdt_api::sdk::{AppState, now_ms};
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{EventType, schema_of};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use ts_rs::TS;

pub const SCRYER_DOWNLOAD_FINISHED: &str = "scryer.download_finished";

pub const EVENT_TYPES: &[EventType] = &[EventType {
    id: SCRYER_DOWNLOAD_FINISHED,
    title: "Scryer download finished",
    description: "Scryer finished a download and imported it into the library (checked every minute while a trigger \
                  is on; turning one on only records what Scryer has already finished).",
    schema: schema_of::<ScryerDownloadFinished>,
    outside: &[],
}];

/// `scryer.download_finished`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ScryerDownloadFinished {
    /// The `scryer` connection.
    pub connection: String,
    /// Scryer's id of the download.
    pub download_id: String,
    /// The title (or the release, when Scryer matched none).
    pub title: String,
    /// `movie`, `series` or `anime`, when Scryer matched a title.
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub kind: Option<String>,
    /// The quality the title has now (`1080p` …), when Scryer says.
    pub quality: Option<String>,
    /// The title's folder in the library, when Scryer matched one.
    pub path: Option<String>,
    /// The download client.
    pub client: String,
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    /// When it was imported (Scryer's UTC timestamp), when known.
    pub finished_at: Option<String>,
}

/// The newest successful downloads (Scryer allows 50 a page).
pub(crate) const HISTORY: &str = "query PdtHistory { downloadHistory(limit: 50, filters: [SUCCESS]) { items { id titleName \
                       facet clientName sizeBytes importedAt title { currentQualityTier rootFolderPath } } } }";

/// A look this long after the last one starts over: what finished meanwhile is not news to anyone.
const STALE_MS: i64 = 10 * 60 * 1000;

fn baseline_key(connection_id: &str) -> String {
    format!("sensor.scryer.{connection_id}")
}

/// A history item as the event's payload.
pub fn finished(connection: &str, v: &Value) -> Option<ScryerDownloadFinished> {
    let s = |v: &Value, k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    Some(ScryerDownloadFinished {
        connection: connection.into(),
        download_id: s(v, "id")?,
        title: s(v, "titleName").unwrap_or_default(),
        kind: s(v, "facet").map(|f| f.to_ascii_lowercase()),
        quality: s(&v["title"], "currentQualityTier"),
        path: s(&v["title"], "rootFolderPath"),
        client: s(v, "clientName").unwrap_or_default(),
        size_bytes: v["sizeBytes"].as_u64(),
        finished_at: s(v, "importedAt"),
    })
}

/// Whether an enabled workflow has the trigger.
async fn watched(st: &AppState) -> bool {
    for wf in st.store.enabled_workflows().await.unwrap_or_default() {
        if let Ok(Some(doc)) = st.store.workflow_doc(&wf.id, wf.version).await
            && doc.nodes.iter().any(|n| n.kind == SCRYER_DOWNLOAD_FINISHED)
        {
            return true;
        }
    }
    false
}

/// One look at every Scryer connection's finished downloads. Never fails: a connection that fails is reported on it,
/// other problems are logged.
pub async fn refresh_scryer(st: &AppState) {
    if !watched(st).await {
        return;
    }
    let conns = match st.store.list_connections().await {
        Ok(c) => c,
        Err(e) => return tracing::warn!(error = %e, "the Scryer check could not list connections"),
    };
    let mut uses = pdt_api::sdk::connections::Uses::default();
    for c in conns.iter().filter(|c| c.kind == SCRYER) {
        match poll(st, &c.id, &c.name).await {
            Ok(()) => uses.worked(&c.name),
            Err(e) => uses.failed(&c.name, &e),
        }
    }
    uses.report(st).await;
}

async fn poll(st: &AppState, id: &str, name: &str) -> Result<(), String> {
    let client = Client::open(st, name).await?;
    let data = client.gql_named(HISTORY, json!({}), None).await?;
    let items: Vec<ScryerDownloadFinished> =
        data["downloadHistory"]["items"].as_array().into_iter().flatten().filter_map(|v| finished(name, v)).collect();
    let key = baseline_key(id);
    let now = now_ms();
    let record = match st.store.get_host_value(&key).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(connection = name, error = %e, "could not read which Scryer downloads were seen");
            return Ok(());
        }
    };
    let seen: Option<BTreeSet<String>> = record
        .filter(|v| v["at"].as_i64().is_some_and(|at| now - at < STALE_MS))
        .and_then(|v| serde_json::from_value(v["ids"].clone()).ok());
    let mut now_seen: BTreeSet<String> = items.iter().map(|i| i.download_id.clone()).collect();
    if let Some(seen) = &seen {
        for i in items.iter().filter(|i| !seen.contains(&i.download_id)) {
            let key = format!("scryer:{id}:{}", i.download_id);
            let e = NewLogEvent::new(SCRYER_DOWNLOAD_FINISHED, key, "scryer", i);
            if let Err(x) = st.store.emit_event(&e, now).await {
                tracing::warn!(connection = name, error = %x, "could not log a finished Scryer download");
                now_seen.remove(&i.download_id);
            }
        }
    }
    if let Err(e) = st.store.put_host_value(&key, &json!({"ids": now_seen, "at": now})).await {
        tracing::warn!(connection = name, error = %e, "could not record which Scryer downloads were seen");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_item_becomes_the_payload() {
        let v = json!({"id": "d1", "titleName": "Dune", "facet": "MOVIE", "clientName": "qbit", "sizeBytes": 42,
                       "importedAt": "2026-11-01T10:00:00Z",
                       "title": {"currentQualityTier": "2160p", "rootFolderPath": "/media/films/Dune (2021)"}});
        let p = finished("home", &v).unwrap();
        assert_eq!((p.title.as_str(), p.kind.as_deref(), p.quality.as_deref()), ("Dune", Some("movie"), Some("2160p")));
        assert_eq!((p.path.as_deref(), p.size_bytes), (Some("/media/films/Dune (2021)"), Some(42)));
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!((&j["type"], &j["downloadId"]), (&json!("movie"), &json!("d1")));
        // Unmatched: no title.
        let p = finished("home", &json!({"id": "d2", "titleName": "Some.Release", "title": null})).unwrap();
        assert_eq!((p.kind, p.quality, p.path), (None, None, None));
        assert!(finished("home", &json!({"titleName": "no id"})).is_none());
    }
}
