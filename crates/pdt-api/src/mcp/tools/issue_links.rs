//! The PM's tools for a ticket's links to issues in other trackers and its plain links (PDT-481).

use super::*;
use pdt_core::fence::Fence;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkIssue {
    /// The ticket's key (`ABC-12`) or id.
    ticket: String,
    /// The issue: its address (`https://github.com/o/r/issues/12`, a Jira or GitLab issue's), or its key
    /// (`owner/name#12`, `ABC-123`). An address no tracker recognizes is added as a plain link.
    issue: String,
    /// A plain link's label (`Docs`, `Figma`); its address's host when left out.
    #[serde(default)]
    label: Option<String>,
    /// Moving the ticket to Done closes (or moves) the issue. Off when left out: someone else's issue is never
    /// closed by surprise.
    #[serde(default)]
    close_on_done: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnlinkIssue {
    /// The ticket's key (`ABC-12`) or id.
    ticket: String,
    /// The issue's key (`owner/name#12`, `ABC-123`) or address, or a plain link's address.
    issue: String,
}

pub(super) fn defs() -> Vec<Tool> {
    vec![
        Tool {
            decl: ToolDecl::new("link_issue", "Link issues in other trackers (GitHub, GitLab, Jira) to tickets")
                .asks()
                .open_world()
                .pm(),
            title: "Link an issue to a ticket",
            description: "Links an issue that is already in a tracker (GitHub, GitLab, Jira…) to a ticket, by its \
                          address or key: it shows beside the ticket's key with its title and state. Done leaves the \
                          issue alone unless closeOnDone. An address no tracker recognizes is added as a plain link \
                          (with `label`). The user confirms each call unless the project lets you act.",
            schema: schema::<LinkIssue>,
        },
        Tool {
            decl: ToolDecl::new("unlink_issue", "Unlink issues and plain links from tickets").asks().pm(),
            title: "Unlink an issue from a ticket",
            description: "Removes a ticket's link to an issue (by its key or address) or a plain link (by its \
                          address). Nothing changes in the tracker. The user confirms each call unless the project \
                          lets you act.",
            schema: schema::<UnlinkIssue>,
        },
    ]
}

/// Wraps the `title` of each issue in `issues` (a ticket's `external`, or [`links_json`]'s `issues`) in `fence`: what
/// a tracker says an issue is called is outside text, whoever linked it (PDT-536). Whether any had one.
pub(super) fn fence_titles(issues: &mut Value, fence: &Fence) -> bool {
    let mut any = false;
    for l in issues.as_array_mut().into_iter().flatten() {
        if let Some(t) = l["title"].as_str() {
            l["title"] = json!(fence.wrap(t));
            any = true;
        }
    }
    any
}

/// What the agent reads back: the ticket's issues and plain links, the issues' titles in `fence`; and whether any
/// had a title.
fn links_json(t: &pdt_core::ticket::Ticket, fence: &Fence) -> (Value, bool) {
    let mut issues: Value = t
        .summary
        .external
        .iter()
        .map(|l| {
            json!({"tracker": l.system, "key": l.key, "url": l.url, "title": l.title, "state": l.state,
                   "origin": l.origin, "closeOnDone": l.close_on_done})
        })
        .collect();
    let fenced = fence_titles(&mut issues, fence);
    let links: Vec<Value> = t.web_links.iter().map(|l| json!({"label": l.label, "url": l.url})).collect();
    (json!({"key": t.summary.key, "issues": issues, "links": links}), fenced)
}

/// A link tool's answer: `v` with the note on `fence`'s blocks (the issues' titles) when it has any (`fenced`).
fn noted(mut v: Value, fence: &Fence, fenced: bool) -> Value {
    if fenced {
        v["outsideText"] = json!(format!(
            "{} An issue's title is what its tracker says, so each is in such a block (the tags are not part of it).",
            fence.note()
        ));
    }
    v
}

/// The issues and plain links `given` names for a new ticket of `project_id` (`create_ticket`'s `external`), read
/// before the ticket is made so a bad one stops it.
pub(super) async fn recognize_all(st: &AppState, project_id: &str, given: &[String]) -> Result<Vec<Found>, String> {
    let mut out = vec![];
    for g in given.iter().map(|g| g.trim()).filter(|g| !g.is_empty()) {
        out.push(recognize_one(st, project_id, g, None).await?);
    }
    Ok(out)
}

/// An issue a tracker recognized, or a plain link.
pub(super) enum Found {
    Issue(crate::issue_links::Recognized),
    Web(String, String),
}

async fn recognize_one(st: &AppState, project_id: &str, given: &str, label: Option<&str>) -> Result<Found, String> {
    match crate::issue_links::recognize(st, project_id, given).await {
        Ok(r) => Ok(Found::Issue(r)),
        Err(crate::issue_links::LinkError::Unrecognized(_)) if pdt_core::issue_link::url_parts(given).is_some() => {
            let (url, label) = pdt_core::issue_link::web_link(given, label.unwrap_or_default())?;
            Ok(Found::Web(url, label))
        }
        Err(e) => Err(e.message(given)),
    }
}

/// Links what was found to ticket `id`, on behalf of `by`.
pub(super) async fn link(
    st: &AppState,
    id: &str,
    found: &Found,
    close_on_done: Option<bool>,
    by: By,
) -> Result<pdt_core::ticket::Ticket, String> {
    match found {
        Found::Issue(r) => crate::issue_links::link_found(st, id, r, close_on_done, by).await.map_err(api),
        Found::Web(url, label) => st.store.add_web_link(id, url, label, by, now_ms()).await.map_err(|e| api(e.into())),
    }
}

pub(super) async fn call(cx: &Cx<'_>, name: &str, a: Value) -> Result<Value, String> {
    let st = cx.st;
    match name {
        "link_issue" => {
            let a: LinkIssue = args(a)?;
            let id = resolve_ticket(st, &a.ticket, Some(&cx.session.project_id)).await?;
            let project =
                st.store.get_ticket_summary(&id).await.map_err(store)?.ok_or("that ticket no longer exists")?;
            confirmable(cx, name, &AskTarget::Ticket(id.clone())).await?;
            let found = recognize_one(st, &project.project_id, &a.issue, a.label.as_deref()).await?;
            let by = agent_by(st, cx.session).await;
            let t = link(st, &id, &found, a.close_on_done, by).await?;
            // The one just linked, for the conversation's card.
            let fence = new_fence();
            let issue = match &found {
                Found::Issue(r) => json!({"key": r.issue.key, "url": r.issue.url, "title": fence.wrap(&r.issue.title)}),
                Found::Web(url, label) => json!({"key": null, "url": url, "title": label}),
            };
            let (ticket, titled) = links_json(&t, &fence);
            let fenced = titled || matches!(found, Found::Issue(_));
            Ok(noted(json!({"linked": true, "issue": issue, "ticket": ticket}), &fence, fenced))
        }
        "unlink_issue" => {
            let a: UnlinkIssue = args(a)?;
            let id = resolve_ticket(st, &a.ticket, Some(&cx.session.project_id)).await?;
            let t = st.store.get_ticket(&id).await.map_err(store)?.ok_or("that ticket no longer exists")?;
            let given = a.issue.trim();
            let issue = t.summary.external.iter().find(|l| l.key.eq_ignore_ascii_case(given) || l.url == given);
            let web = t.web_links.iter().find(|l| l.url == given);
            if issue.is_none() && web.is_none() {
                return Err(format!("{} has no link to {given:?}", t.summary.key));
            }
            confirmable(cx, name, &AskTarget::Ticket(id.clone())).await?;
            let by = agent_by(st, cx.session).await;
            // The one removed, for the conversation's card.
            let fence = new_fence();
            let removed = match (issue, web) {
                (Some(l), _) => json!({"key": l.key, "url": l.url, "title": l.title.as_deref().map(|t| fence.wrap(t))}),
                (None, Some(w)) => json!({"key": null, "url": w.url, "title": w.label}),
                (None, None) => unreachable!("checked above"),
            };
            let t = match (issue, web) {
                (Some(l), _) => st.store.unlink_external(&id, &l.system, &l.key, by, now_ms()).await,
                (None, Some(w)) => st.store.remove_web_link(&id, &w.id, by, now_ms()).await,
                (None, None) => unreachable!("checked above"),
            }
            .map_err(|e| api(e.into()))?;
            let (ticket, titled) = links_json(&t, &fence);
            let fenced = titled || issue.is_some_and(|l| l.title.is_some());
            Ok(noted(json!({"unlinked": true, "issue": removed, "ticket": ticket}), &fence, fenced))
        }
        other => Err(format!("no tool is called {other}")),
    }
}

fn new_fence() -> Fence {
    Fence::new(uuid::Uuid::new_v4().simple().to_string())
}
