//! Linking a ticket to a Jira issue that is already there (PDT-481): an issue's address on a Jira site PDT has a
//! connection to, or its key (`ABC-123`) through the project's Jira connection (or the only one), read for its summary
//! and status.

use crate::client::JiraClient;
use crate::core::{JIRA, JiraSettings, issue_key, issue_state, key_in_url};
use pdt_api::sdk::AppState;
use pdt_api::sdk::issues::{FindError, FoundIssue, IssueNow, StoredExternalLink};
use pdt_core::workflow::api::Connection;

/// The Jira connections.
async fn sites(st: &AppState) -> Result<Vec<Connection>, FindError> {
    Ok(st.store.list_connections().await?.into_iter().filter(|c| c.kind == JIRA).collect())
}

/// The issue `given` names: None when it is neither an address on a Jira site PDT knows nor an issue key (or PDT has
/// no Jira site).
pub(crate) async fn find(st: &AppState, project_id: &str, given: &str) -> Option<Result<FoundIssue, FindError>> {
    let all = match sites(st).await {
        Ok(all) if all.is_empty() => return None,
        Ok(all) => all,
        Err(e) => return Some(Err(e)),
    };
    if let Some((conn, key)) = all.iter().find_map(|c| key_in_url(&c.url, given).map(|k| (c, k))) {
        return Some(read(st, &conn.name, &key).await.map_err(FindError::Tracker));
    }
    let key = issue_key(given)?;
    let linked = match st.store.get_project_settings(project_id).await {
        Ok(s) => s.integration::<JiraSettings>(JIRA).map(|j| j.connection),
        Err(e) => return Some(Err(e.into())),
    };
    let name = match (linked, <[Connection; 1]>::try_from(all)) {
        (Some(n), _) => n,
        (None, Ok([one])) => one.name,
        (None, Err(_)) => {
            return Some(Err(FindError::Tracker(format!(
                "PDT has several Jira sites: give {key}'s address, or link the project to one in its settings"
            ))));
        }
    };
    Some(read(st, &name, &key).await.map_err(FindError::Tracker))
}

async fn read(st: &AppState, connection: &str, key: &str) -> Result<FoundIssue, String> {
    let c = JiraClient::named(st, connection).await?;
    let now = now(&c, key).await?;
    Ok(FoundIssue {
        connection_id: c.conn.id.clone(),
        key: key.to_string(),
        url: c.browse(key),
        title: now.title,
        state: now.state,
    })
}

async fn now(c: &JiraClient, key: &str) -> Result<IssueNow, String> {
    let v = c
        .issue_raw(key, &["summary".into(), "status".into()])
        .await?
        .ok_or_else(|| format!("Jira has no issue {key} (or {} can't see it)", c.conn.name))?;
    let title = v["fields"]["summary"].as_str().unwrap_or(key).to_string();
    Ok(IssueNow { title, state: issue_state(&v["fields"]) })
}

/// What Jira says now of a linked issue, through the connection it was linked with.
pub(crate) async fn read_link(st: &AppState, link: &StoredExternalLink) -> Result<IssueNow, String> {
    let conn = st
        .store
        .get_connection(&link.connection_id)
        .await
        .map_err(pdt_api::sdk::err)?
        .ok_or("the Jira connection the issue was linked with is gone")?;
    let c = JiraClient::named(st, &conn.name).await?;
    now(&c, &link.key).await
}
