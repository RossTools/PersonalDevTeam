//! How the sensors' uses of connections go (PDT-130). A sensor that cannot use a connection (a revoked token, the
//! service down, a connection that is gone) records why on the connection (`lastError`, shown in Settings →
//! Connections) and makes one `connection.failed` notification for it; the first use that works again clears both.
//! Saving the connection clears them too (the next use tells whether that fixed it).

use crate::routes::{AppState, now_ms};
use pdt_core::notification::{NewNotification, NotificationKind, NotificationTarget, Severity, connection_key};
use pdt_core::workflow::api::NewLogEvent;
use pdt_core::workflow::events::{CONNECTION_FAILED, ConnectionFailed};
use std::collections::BTreeMap;

/// One sensor pass's uses of connections, by name: the first failure of each, or None when every use worked.
#[derive(Debug, Default)]
pub struct Uses(BTreeMap<String, Option<String>>);

impl Uses {
    pub fn worked(&mut self, name: &str) {
        self.0.entry(name.to_string()).or_insert(None);
    }

    pub fn failed(&mut self, name: &str, error: &str) {
        let e = self.0.entry(name.to_string()).or_insert(None);
        if e.is_none() {
            *e = Some(error.to_string());
        }
    }

    /// Records the pass on each connection (failing if any use of it failed): notifies or clears once per change.
    pub async fn report(self, st: &AppState) {
        for (name, error) in self.0 {
            report(st, &name, error.as_deref()).await;
        }
    }
}

/// Records one connection's state now: `error` when a use failed, None when it worked.
pub async fn report(st: &AppState, name: &str, error: Option<&str>) {
    let key = connection_key(name);
    let conn = match st.store.connection_by_name(name).await {
        Ok(c) => c,
        Err(e) => return tracing::warn!(connection = name, error = %e, "could not read a connection"),
    };
    let changed = match &conn {
        Some(c) => match st.store.set_connection_error(&c.id, error, now_ms()).await {
            Ok(changed) => changed,
            Err(e) => return tracing::warn!(connection = name, error = %e, "could not record a connection's state"),
        },
        // No row to keep the state on: news unless its notification is already live.
        None => match st.store.live_notifications().await {
            Ok(live) => error.is_some() && !live.iter().any(|n| n.dedupe_key == key),
            Err(_) => false,
        },
    };
    if !changed {
        return;
    }
    match error {
        Some(e) => {
            tracing::warn!(connection = name, error = e, "a connection is failing");
            // Only when it starts failing: another error while it fails is the same failure.
            if conn.as_ref().is_none_or(|c| c.last_error.is_none()) {
                let kind = conn.as_ref().map(|c| c.kind.clone()).unwrap_or_default();
                log_failed(st, name, &kind, "connection", e, &format!("connection:{name}:{}", now_ms())).await;
            }
            let mut n = NewNotification::new(
                NotificationKind::ConnectionFailed,
                key,
                format!("{name} is failing"),
                NotificationTarget::Connections,
            );
            n.severity = Severity::Warning;
            n.body = e.to_string();
            crate::notifications::notify(st, n).await;
        }
        None => {
            tracing::info!(connection = name, "a connection works again");
            crate::notifications::clear(st, &key).await;
        }
    }
}

/// `connection.failed` (PDT-365): `name` (of `kind`, from `source`: `connection` or `forge`) started failing with
/// `error`. `key` names this failure, so it is logged once.
pub(crate) async fn log_failed(st: &AppState, name: &str, kind: &str, source: &str, error: &str, key: &str) {
    let payload = ConnectionFailed { name: name.into(), kind: kind.into(), source: source.into(), error: error.into() };
    let e = NewLogEvent::new(CONNECTION_FAILED, key, "connections", payload);
    if let Err(x) = st.store.emit_event(&e, now_ms()).await {
        tracing::warn!(connection = name, error = %x, "could not log a failing connection");
    }
}

/// A connection was saved, renamed or deleted: its old failure is forgotten (the next use tells).
pub async fn forget(st: &AppState, names: &[&str]) {
    for n in names {
        crate::notifications::clear(st, &connection_key(n)).await;
    }
}
