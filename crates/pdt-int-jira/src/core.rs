//! Jira Cloud (Phase 7e spec §6, PDT-85): issues as REST v3 returns them, Atlassian Document Format (ADF) to and from
//! text, and what is news about an issue since a poll. Pure: [`crate::sensor`] calls Jira and logs the events.

use pdt_core::workflow::events::{EventType, schema_of};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// The connection kind, and the `system` of a ticket's link to a Jira issue.
pub const JIRA: &str = "jira";

/// Jira's event types (the `jira` integration's), in catalog order.
pub const EVENT_TYPES: &[EventType] = &[
    EventType {
        id: JIRA_ISSUE_CREATED,
        title: "Jira issue created",
        description: "A new issue in a project's Jira scope (checked every minute; linking a project only records where \
                      it is).",
        schema: schema_of::<JiraIssueCreated>,
        outside: &["summary", "description"],
    },
    EventType {
        id: JIRA_ISSUE_UPDATED,
        title: "Jira issue updated",
        description: "Someone changed fields of an issue in a project's Jira scope (`changed` names them).",
        schema: schema_of::<JiraIssueUpdated>,
        outside: &["summary", "description"],
    },
    EventType {
        id: JIRA_ISSUE_TRANSITIONED,
        title: "Jira issue transitioned",
        description: "An issue in a project's Jira scope moved to another status.",
        schema: schema_of::<JiraIssueTransitioned>,
        outside: &["summary", "description"],
    },
    EventType {
        id: JIRA_ISSUE_COMMENTED,
        title: "Jira issue commented",
        description: "Someone commented on an issue in a project's Jira scope.",
        schema: schema_of::<JiraIssueCommented>,
        outside: &["summary", "description", "body"],
    },
];

/// The transition (or status) a linked ticket's Done moves its issue to, unless the project says otherwise.
pub const DEFAULT_DONE: &str = "Done";

/// The fields a poll or an import asks Jira for.
pub const FIELDS: &[&str] = &[
    "summary",
    "description",
    "status",
    "assignee",
    "reporter",
    "issuetype",
    "priority",
    "labels",
    "created",
    "updated",
    "comment",
];

/// A project's link to Jira on this host (its settings, not the repo's): its `jira` integration settings, read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraSettings {
    /// A `jira` connection's name.
    pub connection: String,
    /// JQL for the project's issues (`project = ABC`); empty: every issue the account can see.
    #[serde(default)]
    pub scope: String,
    /// The transition or status a ticket moved to Done moves its issue to.
    #[serde(default = "default_done")]
    pub done: String,
}

fn default_done() -> String {
    DEFAULT_DONE.into()
}

/// One issue, as PDT reads it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JiraIssue {
    pub key: String,
    /// `<site>/browse/<key>`.
    pub url: String,
    pub summary: String,
    /// Text from the ADF description.
    pub description: String,
    pub status: String,
    pub assignee: Option<String>,
    pub assignee_id: Option<String>,
    pub reporter: Option<String>,
    pub issue_type: String,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    /// ms since the epoch.
    pub created: i64,
    pub updated: i64,
    /// `updated` as Jira wrote it (part of an update's event key).
    pub updated_raw: String,
    pub histories: Vec<History>,
    pub comments: Vec<JiraComment>,
}

/// One changelog entry: the fields one edit changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    pub id: String,
    pub created: i64,
    pub items: Vec<HistoryItem>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HistoryItem {
    pub field: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct JiraComment {
    pub id: String,
    pub author: String,
    pub author_id: Option<String>,
    /// Text from the ADF body.
    pub body: String,
    pub created: i64,
}

/// What a poll found new about an issue since it last looked.
#[derive(Debug, Clone, PartialEq)]
pub enum IssueNews {
    Created,
    /// Fields an edit changed (by name, as the changelog has them).
    Updated {
        changed: Vec<String>,
    },
    Transitioned {
        history: String,
        from: String,
        to: String,
    },
    Commented(JiraComment),
}

/// Jira's timestamps (`2026-10-22T10:00:00.000+0000`, also RFC 3339) as ms since the epoch.
pub fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z")
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(s))
        .ok()
        .map(|t| t.timestamp_millis())
}

/// An issue key (`ABC-123`, any case, trimmed) in Jira's form, or None.
pub fn issue_key(s: &str) -> Option<String> {
    let k = s.trim().to_ascii_uppercase();
    let (project, number) = k.rsplit_once('-')?;
    let project_ok = project.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && project.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let number_ok = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    (project_ok && number_ok).then_some(k)
}

/// The site's base URL without a trailing slash.
pub fn site(url: &str) -> &str {
    url.trim().trim_end_matches('/')
}

fn text(v: &Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str().map(String::from)
}

/// An issue from REST v3 JSON (`fields` with at least [`FIELDS`]; `changelog` when expanded). None without a key.
pub fn parse_issue(site_url: &str, v: &Value) -> Option<JiraIssue> {
    let key = v.get("key")?.as_str()?.to_string();
    let f = &v["fields"];
    let histories = v["changelog"]["histories"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|h| History {
                    id: h["id"].as_str().map(String::from).unwrap_or_else(|| h["id"].to_string()),
                    created: h["created"].as_str().and_then(parse_time).unwrap_or(0),
                    items: h["items"]
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .map(|i| HistoryItem {
                                    field: i["field"].as_str().unwrap_or_default().to_string(),
                                    from: i["fromString"].as_str().map(String::from),
                                    to: i["toString"].as_str().map(String::from),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    let comments = f["comment"]["comments"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|c| JiraComment {
                    id: c["id"].as_str().map(String::from).unwrap_or_else(|| c["id"].to_string()),
                    author: text(c, &["author", "displayName"]).unwrap_or_default(),
                    author_id: text(c, &["author", "accountId"]).or_else(|| text(c, &["author", "name"])),
                    body: adf_to_text(&c["body"]),
                    created: c["created"].as_str().and_then(parse_time).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();
    let updated_raw = f["updated"].as_str().unwrap_or_default().to_string();
    Some(JiraIssue {
        url: format!("{}/browse/{key}", site(site_url)),
        key,
        summary: f["summary"].as_str().unwrap_or_default().to_string(),
        description: adf_to_text(&f["description"]),
        status: text(f, &["status", "name"]).unwrap_or_default(),
        assignee: text(f, &["assignee", "displayName"]),
        assignee_id: text(f, &["assignee", "accountId"]).or_else(|| text(f, &["assignee", "name"])),
        reporter: text(f, &["reporter", "displayName"]),
        issue_type: text(f, &["issuetype", "name"]).unwrap_or_default(),
        priority: text(f, &["priority", "name"]),
        labels: f["labels"]
            .as_array()
            .map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        created: f["created"].as_str().and_then(parse_time).unwrap_or(0),
        updated: parse_time(&updated_raw).unwrap_or(0),
        updated_raw,
        histories,
        comments,
    })
}

/// What changed about `issue` after `since` (ms): created, the fields edits changed, each status change, each new
/// comment. A newly created issue's own edits are not separate news.
pub fn news(issue: &JiraIssue, since: i64) -> Vec<IssueNews> {
    let mut out = vec![];
    let created = issue.created >= since;
    if created {
        out.push(IssueNews::Created);
    }
    let recent: Vec<&History> = issue.histories.iter().filter(|h| h.created >= since).collect();
    if !created {
        let mut changed: Vec<String> = vec![];
        for i in recent.iter().flat_map(|h| &h.items) {
            if !changed.contains(&i.field) {
                changed.push(i.field.clone());
            }
        }
        if !changed.is_empty() {
            out.push(IssueNews::Updated { changed });
        }
    }
    let mut transitions: Vec<&History> =
        recent.iter().copied().filter(|h| h.items.iter().any(|i| i.field == "status")).collect();
    transitions.sort_by_key(|h| h.created);
    for h in transitions {
        let i = h.items.iter().find(|i| i.field == "status").expect("filtered on a status item");
        out.push(IssueNews::Transitioned {
            history: h.id.clone(),
            from: i.from.clone().unwrap_or_default(),
            to: i.to.clone().unwrap_or_default(),
        });
    }
    let mut comments: Vec<&JiraComment> = issue.comments.iter().filter(|c| c.created >= since).collect();
    comments.sort_by_key(|c| c.created);
    out.extend(comments.into_iter().map(|c| IssueNews::Commented(c.clone())));
    out
}

/// The JQL of one poll: the project's scope, issues updated in the last `minutes` (relative, because JQL dates are
/// in the Jira user's time zone), oldest first.
pub fn poll_jql(scope: &str, minutes: i64) -> String {
    let window = format!("updated >= \"-{}m\"", minutes.max(1));
    match scope.trim() {
        "" => format!("{window} ORDER BY updated ASC"),
        s => format!("({s}) AND {window} ORDER BY updated ASC"),
    }
}

/// Text from an ADF document (a description or comment): paragraphs and headings as lines, lists as `- ` items,
/// mentions and links as their text. Anything else gives its children's text. Not ADF (a plain string): as it is.
pub fn adf_to_text(v: &Value) -> String {
    fn inline(v: &Value, out: &mut String) {
        match v["type"].as_str().unwrap_or_default() {
            "text" => out.push_str(v["text"].as_str().unwrap_or_default()),
            "hardBreak" => out.push('\n'),
            "mention" | "emoji" | "status" | "date" => {
                let a = &v["attrs"];
                out.push_str(a["text"].as_str().or(a["shortName"].as_str()).unwrap_or_default())
            }
            "inlineCard" | "blockCard" | "embedCard" => out.push_str(v["attrs"]["url"].as_str().unwrap_or_default()),
            _ => children(v, out),
        }
    }
    fn children(v: &Value, out: &mut String) {
        for c in v["content"].as_array().into_iter().flatten() {
            inline(c, out);
        }
    }
    fn blocks(v: &Value, prefix: &str, out: &mut Vec<String>) {
        for (n, b) in v["content"].as_array().into_iter().flatten().enumerate() {
            match b["type"].as_str().unwrap_or_default() {
                "bulletList" => blocks(b, &format!("{}- ", " ".repeat(prefix.len())), out),
                "orderedList" => {
                    for (i, item) in b["content"].as_array().into_iter().flatten().enumerate() {
                        blocks(item, &format!("{}{}. ", " ".repeat(prefix.len()), i + 1), out);
                    }
                }
                "listItem" | "blockquote" | "panel" | "expand" | "layoutSection" | "layoutColumn" | "table"
                | "tableRow" | "tableCell" | "tableHeader" => blocks(b, prefix, out),
                "codeBlock" => {
                    let mut s = String::new();
                    children(b, &mut s);
                    out.push(format!("{prefix}```\n{s}\n```"));
                }
                "rule" => out.push("---".into()),
                _ => {
                    let mut s = String::new();
                    inline(b, &mut s);
                    let lead = if prefix.is_empty() || n == 0 { prefix.to_string() } else { " ".repeat(prefix.len()) };
                    out.push(format!("{lead}{s}"));
                }
            }
        }
    }
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => {
            let mut out = vec![];
            blocks(v, "", &mut out);
            let mut text = String::new();
            let mut prev_item = false;
            for line in out {
                let start = line.trim_start();
                let item =
                    start.starts_with("- ") || start.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok());
                if !text.is_empty() {
                    text.push_str(if item && prev_item { "\n" } else { "\n\n" });
                }
                text.push_str(&line);
                prev_item = item;
            }
            text.trim().to_string()
        }
    }
}

/// An ADF document from text: a paragraph per blank-line-separated block, line breaks kept.
pub fn text_to_adf(s: &str) -> Value {
    let paragraphs: Vec<Value> = s
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(|p| p.trim_matches('\n'))
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let mut content = vec![];
            for (i, line) in p.split('\n').enumerate() {
                if i > 0 {
                    content.push(json!({"type": "hardBreak"}));
                }
                if !line.is_empty() {
                    content.push(json!({"type": "text", "text": line}));
                }
            }
            json!({"type": "paragraph", "content": content})
        })
        .collect();
    json!({"type": "doc", "version": 1, "content": paragraphs})
}

pub const JIRA_ISSUE_CREATED: &str = "jira.issue.created";
pub const JIRA_ISSUE_UPDATED: &str = "jira.issue.updated";
pub const JIRA_ISSUE_TRANSITIONED: &str = "jira.issue.transitioned";
pub const JIRA_ISSUE_COMMENTED: &str = "jira.issue.commented";

/// The issue every `jira.issue.*` event is about.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueRef {
    /// The PDT project whose Jira scope it is in.
    pub project_id: String,
    /// The `jira` connection's name.
    pub connection: String,
    /// `ABC-123`.
    pub key: String,
    pub url: String,
    pub summary: String,
    /// Text (from Jira's rich text).
    pub description: String,
    pub status: String,
    pub assignee: Option<String>,
    /// Assigned to the account the connection uses (the user).
    pub assigned_to_me: bool,
    pub reporter: Option<String>,
    pub issue_type: String,
    pub priority: Option<String>,
    pub labels: Vec<String>,
    /// The PDT ticket linked to it, if any.
    pub ticket_id: Option<String>,
}

/// `jira.issue.created`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueCreated {
    #[serde(flatten)]
    #[ts(flatten)]
    pub issue: JiraIssueRef,
}

/// `jira.issue.updated`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueUpdated {
    #[serde(flatten)]
    #[ts(flatten)]
    pub issue: JiraIssueRef,
    /// The fields that changed, as Jira names them (`assignee`, `status`, `priority`, …).
    pub changed: Vec<String>,
}

/// `jira.issue.transitioned`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueTransitioned {
    #[serde(flatten)]
    #[ts(flatten)]
    pub issue: JiraIssueRef,
    pub from: String,
    pub to: String,
}

/// `jira.issue.commented`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JiraIssueCommented {
    #[serde(flatten)]
    #[ts(flatten)]
    pub issue: JiraIssueRef,
    pub comment_id: String,
    pub author: String,
    /// Text (from Jira's rich text).
    pub body: String,
    /// Written by the account the connection uses (the user).
    pub by_me: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_json() -> Value {
        json!({
            "key": "ABC-7",
            "fields": {
                "summary": "Fix login",
                "description": {"type": "doc", "version": 1, "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "It fails "},
                        {"type": "mention", "attrs": {"text": "@Ann"}}]},
                    {"type": "bulletList", "content": [
                        {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]},
                        {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "two"}]}]}]}]},
                "status": {"name": "In Progress"},
                "assignee": {"displayName": "Me", "accountId": "acc-me"},
                "reporter": {"displayName": "Ann"},
                "issuetype": {"name": "Bug"},
                "priority": {"name": "High"},
                "labels": ["web"],
                "created": "2026-10-22T09:00:00.000+0000",
                "updated": "2026-10-22T10:05:00.000+0000",
                "comment": {"comments": [
                    {"id": "100", "author": {"displayName": "Ann", "accountId": "acc-ann"},
                     "body": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "old"}]}]},
                     "created": "2026-10-22T09:10:00.000+0000"},
                    {"id": "101", "author": {"displayName": "Ann", "accountId": "acc-ann"},
                     "body": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "new"}]}]},
                     "created": "2026-10-22T10:04:00.000+0000"}]}
            },
            "changelog": {"histories": [
                {"id": "5", "created": "2026-10-22T09:30:00.000+0000", "items": [{"field": "priority", "fromString": "Low", "toString": "High"}]},
                {"id": "6", "created": "2026-10-22T10:01:00.000+0000", "items": [
                    {"field": "status", "fromString": "To Do", "toString": "In Progress"},
                    {"field": "assignee", "fromString": null, "toString": "Me"}]}]}
        })
    }

    #[test]
    fn an_issue_reads_from_rest_v3() {
        let i = parse_issue("https://acme.atlassian.net/", &issue_json()).unwrap();
        assert_eq!(i.url, "https://acme.atlassian.net/browse/ABC-7");
        assert_eq!((i.summary.as_str(), i.status.as_str(), i.issue_type.as_str()), ("Fix login", "In Progress", "Bug"));
        assert_eq!(i.description, "It fails @Ann\n\n- one\n- two");
        assert_eq!((i.assignee.as_deref(), i.assignee_id.as_deref()), (Some("Me"), Some("acc-me")));
        assert_eq!(i.labels, ["web"]);
        assert_eq!(i.created, parse_time("2026-10-22T09:00:00Z").unwrap());
        assert_eq!(i.histories.len(), 2);
        assert_eq!(i.comments[1].body, "new");
        assert!(parse_issue("x", &json!({"fields": {}})).is_none());
    }

    #[test]
    fn news_since_a_poll() {
        let i = parse_issue("https://s", &issue_json()).unwrap();
        let since = parse_time("2026-10-22T10:00:00.000+0000").unwrap();
        let news = news(&i, since);
        assert_eq!(news.len(), 3, "{news:?}");
        assert_eq!(news[0], IssueNews::Updated { changed: vec!["status".into(), "assignee".into()] });
        assert_eq!(
            news[1],
            IssueNews::Transitioned { history: "6".into(), from: "To Do".into(), to: "In Progress".into() }
        );
        assert!(
            matches!(&news[2], IssueNews::Commented(c) if c.id == "101" && c.author_id.as_deref() == Some("acc-ann"))
        );
        // Created since: no separate update news for its first edits.
        let early = parse_time("2026-10-22T08:00:00Z").unwrap();
        let n = super::news(&i, early);
        assert_eq!(n[0], IssueNews::Created);
        assert!(!n.iter().any(|x| matches!(x, IssueNews::Updated { .. })));
        assert_eq!(n.iter().filter(|x| matches!(x, IssueNews::Commented(_))).count(), 2);
        assert!(super::news(&i, parse_time("2026-10-23T00:00:00Z").unwrap()).is_empty());
    }

    #[test]
    fn times_keys_and_jql() {
        assert_eq!(parse_time("2026-10-22T10:00:00.000+0100"), parse_time("2026-10-22T09:00:00Z"));
        assert!(parse_time("nope").is_none());
        assert_eq!(issue_key(" abc-12 ").as_deref(), Some("ABC-12"));
        assert_eq!(issue_key("A2_B-1").as_deref(), Some("A2_B-1"));
        for bad in ["ABC", "ABC-", "-1", "1A-2", "ABC-1x", "AB C-1"] {
            assert!(issue_key(bad).is_none(), "{bad}");
        }
        assert_eq!(poll_jql("project = ABC", 3), "(project = ABC) AND updated >= \"-3m\" ORDER BY updated ASC");
        assert_eq!(poll_jql(" ", 0), "updated >= \"-1m\" ORDER BY updated ASC");
    }

    #[test]
    fn adf_round_trips_plain_text() {
        let doc = text_to_adf("Hello\nthere\n\nSecond");
        assert_eq!(doc["content"].as_array().unwrap().len(), 2);
        assert_eq!(doc["content"][0]["content"][1]["type"], "hardBreak");
        assert_eq!(adf_to_text(&doc), "Hello\nthere\n\nSecond");
        assert_eq!(adf_to_text(&Value::Null), "");
        assert_eq!(adf_to_text(&json!("plain")), "plain");
        let ordered = json!({"type": "doc", "content": [{"type": "orderedList", "content": [
            {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]},
            {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "b"}]}]}]},
            {"type": "codeBlock", "content": [{"type": "text", "text": "x = 1"}]}]});
        assert_eq!(adf_to_text(&ordered), "1. a\n2. b\n\n```\nx = 1\n```");
    }
}
