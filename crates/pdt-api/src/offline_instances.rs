//! Remote instances offline with work waiting (PDT-636): pickup keeps a ticket placed on an offline instance in To do
//! ("waiting for win-box (offline)") until it is back. Once the instance has been offline for
//! [`pdt_core::link::OFFLINE_NOTIFY_AFTER_MS`] with tickets still waiting for it, the owner notifies the user, once
//! per absence; the notification clears when it is back or nothing waits for it any more.

use crate::routes::{AppState, now_ms};
use pdt_core::link::{LinkState, RemoteInstance};
use pdt_core::notification::{NewNotification, NotificationKind, NotificationTarget, Severity};

fn key(link_id: &str) -> String {
    format!("instance-offline:{link_id}")
}

/// The tickets pickup holds for `i` lately (their ids), oldest hold first.
fn waiting_for(st: &AppState, i: &RemoteInstance, now: i64) -> Vec<String> {
    let holds = st.pickup_holds.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<(&String, i64)> = holds
        .iter()
        .filter(|(_, h)| {
            h.project_id == i.project_id
                && h.waits_for.as_deref() == Some(i.instance_id.as_str())
                && now - h.at < crate::scheduler::PICKUP_HOLD_SHOWN_MS
        })
        .map(|(id, h)| (id, h.at))
        .collect();
    out.sort_by_key(|(id, at)| (*at, (*id).clone()));
    out.into_iter().map(|(id, _)| id.clone()).collect()
}

/// Notifies about each of `instances` offline too long with work waiting, and clears the others' notifications.
pub(crate) async fn check(st: &AppState, instances: &[RemoteInstance]) {
    let now = now_ms();
    for i in instances {
        let offline = pdt_core::link::state(i.last_seen_at, true, false, now) == LinkState::Offline;
        let since = i.last_seen_at.unwrap_or(i.created_at);
        let waiting = if offline && now - since >= pdt_core::link::OFFLINE_NOTIFY_AFTER_MS {
            waiting_for(st, i, now)
        } else {
            vec![]
        };
        let Some(first) = waiting.first() else {
            crate::notifications::clear(st, &key(&i.link_id)).await;
            continue;
        };
        // Once per absence: a live one stays as it is.
        let live = st.store.live_notifications().await.map(|l| l.iter().any(|x| x.dedupe_key == key(&i.link_id)));
        if !matches!(live, Ok(false)) {
            continue;
        }
        let n = waiting.len();
        let mut note = NewNotification::new(
            NotificationKind::ConnectionFailed,
            key(&i.link_id),
            format!("{} is offline with work waiting", i.name),
            NotificationTarget::Ticket { id: first.clone(), project_id: i.project_id.clone() },
        );
        note.severity = Severity::Warning;
        note.project_id = Some(i.project_id.clone());
        note.body = format!(
            "{n} ticket{} placed on {} {} waiting for it to come back.",
            if n == 1 { "" } else { "s" },
            i.name,
            if n == 1 { "is" } else { "are" }
        );
        crate::notifications::notify(st, note).await;
    }
}
