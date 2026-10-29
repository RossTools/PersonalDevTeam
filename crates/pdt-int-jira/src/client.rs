//! A `jira` connection ready to call (PDT-85, PDT-258): Jira Cloud with the account's email and API token (basic
//! auth, REST v3), or Jira Server / Data Center with a personal access token (a bearer token, REST v2); the Agile API
//! (boards, sprints, the backlog) is the same on both. The token never leaves the request: errors name the call and
//! Jira's message, never a header.

use crate::core::{FIELDS, JIRA, JiraIssue, parse_issue, site};
use crate::meta::{self, Deployment, FieldMeta, Mappings, Sprint, Transition};
use pdt_api::sdk::AppState;
use pdt_api::sdk::http::{client, read_body, send_error};
use pdt_core::workflow::api::Connection;
use reqwest::Method;
use serde_json::{Map, Value, json};

/// The most pages one read takes (of [`PAGE`] issues); the sensor's next poll reads on from where it stopped.
pub(crate) const MAX_PAGES: usize = 5;
pub(crate) const PAGE: usize = 100;

const AGILE: &str = "/rest/agile/1.0";

enum Auth {
    /// Cloud: the Atlassian account's email and an API token.
    Basic { email: String, token: String },
    /// Server / Data Center: a personal access token.
    Bearer(String),
}

pub(crate) struct JiraClient {
    pub conn: Connection,
    pub deployment: Deployment,
    pub mappings: Mappings,
    auth: Auth,
    /// The longest one call may take (`AppState::sensor_timeout`).
    timeout: std::time::Duration,
}

/// A page of a search: the issues as Jira gave them, where the next page starts, and how many there are when Jira
/// says.
pub(crate) struct SearchPage {
    pub issues: Vec<Value>,
    pub next: Option<String>,
    pub total: Option<i64>,
}

/// Jira's custom fields this site uses for epics and sprints.
#[derive(Debug, Clone, Default)]
pub(crate) struct Special {
    pub epic_name: Option<String>,
    pub epic_link: Option<String>,
    pub sprint: Option<String>,
}

impl JiraClient {
    /// The client of the connection called `name`, or why not.
    pub(crate) async fn named(st: &AppState, name: &str) -> Result<JiraClient, String> {
        let (conn, secret) = pdt_api::sdk::connections::connection_with_secret(st, name).await?;
        Self::of(st, conn, secret)
    }

    pub(crate) fn of(st: &AppState, conn: Connection, secret: Option<String>) -> Result<JiraClient, String> {
        if conn.kind != JIRA {
            return Err(format!("{} is a {} connection, not {JIRA}", conn.name, conn.kind));
        }
        let deployment = Deployment::of(&conn.config);
        if site(&conn.url).is_empty() {
            let example = match deployment {
                Deployment::Cloud => "https://<site>.atlassian.net",
                Deployment::Server => "https://jira.example.com",
            };
            return Err(format!("{} has no site address ({example})", conn.name));
        }
        let token = secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let auth = match deployment {
            Deployment::Cloud => {
                let email = conn.config.get("email").and_then(|e| e.as_str()).unwrap_or_default().trim().to_string();
                if email.is_empty() {
                    return Err(format!("{} names no email to sign in with", conn.name));
                }
                Auth::Basic { email, token: token.ok_or_else(|| format!("{} has no API token", conn.name))? }
            }
            Deployment::Server => {
                Auth::Bearer(token.ok_or_else(|| format!("{} has no personal access token", conn.name))?)
            }
        };
        let mappings = Mappings::of(&conn.config);
        Ok(JiraClient { conn, deployment, mappings, auth, timeout: st.sensor_timeout })
    }

    pub(crate) fn site(&self) -> &str {
        site(&self.conn.url)
    }

    pub(crate) fn browse(&self, key: &str) -> String {
        format!("{}/browse/{key}", self.site())
    }

    /// A REST API path (`/issue/ABC-1`) under this deployment's version.
    fn api(&self, path: &str) -> String {
        format!("{}{path}", self.deployment.api())
    }

    /// One REST call: the JSON answer (`Null` for an empty one), `Ok(None)` for a 404.
    pub(crate) async fn call(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Option<Value>, String> {
        let url = format!("{}{path}", self.site());
        let mut req = client().request(method.clone(), &url).header("accept", "application/json").timeout(self.timeout);
        req = match &self.auth {
            Auth::Basic { email, token } => req.basic_auth(email, Some(token)),
            Auth::Bearer(token) => req.bearer_auth(token),
        };
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| format!("Jira {method} {path}: {}", send_error(e)))?;
        let status = resp.status().as_u16();
        let bytes = read_body(resp, 8 * 1024 * 1024).await?;
        if status == 404 {
            return Ok(None);
        }
        let v: Value =
            if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
        if !(200..300).contains(&status) {
            let mut msg = v["errorMessages"]
                .as_array()
                .and_then(|m| m.first())
                .and_then(|m| m.as_str())
                .map(String::from)
                .or_else(|| {
                    v["errors"].as_object().filter(|o| !o.is_empty()).map(|o| Value::Object(o.clone()).to_string())
                })
                .or_else(|| v["message"].as_str().map(String::from))
                .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
            if status == 401 {
                msg.push_str(match self.deployment {
                    Deployment::Cloud => " (check the connection's email and API token)",
                    Deployment::Server => " (check the connection's personal access token)",
                });
            }
            return Err(format!("Jira answered {status} to {method} {path}: {msg}"));
        }
        Ok(Some(v))
    }

    async fn found(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value, String> {
        self.call(method, path, body).await?.ok_or_else(|| format!("Jira has no {path}"))
    }

    /// The account the connection signs in as: its raw `myself` answer.
    pub(crate) async fn myself_raw(&self) -> Result<Value, String> {
        self.found(Method::GET, &self.api("/myself"), None).await
    }

    /// The account's id (Cloud's `accountId`, Server's username).
    pub(crate) async fn myself(&self) -> Result<String, String> {
        let v = self.myself_raw().await?;
        self.deployment.user_id(&v).ok_or_else(|| "Jira did not say who the account is".into())
    }

    /// `GET /serverInfo` (version, deployment type), when Jira answers it.
    pub(crate) async fn server_info(&self) -> Option<Value> {
        self.call(Method::GET, &self.api("/serverInfo"), None).await.ok().flatten()
    }

    /// One page of the issues `jql` finds. `cursor`: where the page starts (from the last page's `next`).
    pub(crate) async fn search_page(
        &self,
        jql: &str,
        fields: &[String],
        expand: bool,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<SearchPage, String> {
        match self.deployment {
            Deployment::Cloud => {
                let mut body = json!({"jql": jql, "fields": fields, "maxResults": limit});
                if expand {
                    body["expand"] = json!("changelog");
                }
                if let Some(t) = cursor {
                    body["nextPageToken"] = json!(t);
                }
                let v = self.found(Method::POST, "/rest/api/3/search/jql", Some(&body)).await?;
                let next = v["nextPageToken"].as_str().filter(|_| v["isLast"].as_bool() != Some(true));
                Ok(SearchPage {
                    issues: v["issues"].as_array().cloned().unwrap_or_default(),
                    next: next.map(String::from),
                    total: v["total"].as_i64(),
                })
            }
            Deployment::Server => {
                let start: i64 = match cursor {
                    Some(c) => c.trim().parse().map_err(|_| format!("{c:?} is not a page of this search"))?,
                    None => 0,
                };
                let mut body = json!({"jql": jql, "fields": fields, "startAt": start, "maxResults": limit});
                if expand {
                    body["expand"] = json!(["changelog"]);
                }
                let v = self.found(Method::POST, "/rest/api/2/search", Some(&body)).await?;
                let issues = v["issues"].as_array().cloned().unwrap_or_default();
                let total = v["total"].as_i64();
                let end = v["startAt"].as_i64().unwrap_or(start) + issues.len() as i64;
                let more = !issues.is_empty() && total.is_some_and(|t| end < t);
                Ok(SearchPage { issues, next: more.then(|| end.to_string()), total })
            }
        }
    }

    /// The issues `jql` finds, with their changelog (up to [`MAX_PAGES`] pages), and whether there were more.
    pub(crate) async fn search(&self, jql: &str) -> Result<(Vec<Value>, bool), String> {
        let fields: Vec<String> = FIELDS.iter().map(|f| f.to_string()).collect();
        let mut out = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = self.search_page(jql, &fields, true, PAGE, cursor.as_deref()).await?;
            out.extend(page.issues);
            match page.next {
                Some(n) => cursor = Some(n),
                None => return Ok((out, false)),
            }
        }
        Ok((out, true))
    }

    /// Fills in the newest changelog entries and comments of a search result whose lists Jira cut short (a search
    /// returns at most 100 of each, oldest first), so recent ones are never missed.
    pub(crate) async fn complete(&self, v: &mut Value) -> Result<(), String> {
        let key = v["key"].as_str().unwrap_or_default().to_string();
        let short = |list: &Value, total: &Value| {
            total.as_u64().is_some_and(|t| t > list.as_array().map_or(0, |l| l.len()) as u64)
        };
        // Server's search gives the whole changelog; Cloud's has an endpoint for the rest.
        if self.deployment == Deployment::Cloud && short(&v["changelog"]["histories"], &v["changelog"]["total"]) {
            let total = v["changelog"]["total"].as_u64().unwrap_or(0);
            let start = total.saturating_sub(PAGE as u64);
            let path = format!("/rest/api/3/issue/{key}/changelog?startAt={start}&maxResults={PAGE}");
            if let Some(page) = self.call(Method::GET, &path, None).await? {
                v["changelog"]["histories"] = page["values"].clone();
            }
        }
        if short(&v["fields"]["comment"]["comments"], &v["fields"]["comment"]["total"]) {
            let path = self.api(&format!("/issue/{key}/comment?orderBy=-created&maxResults={PAGE}"));
            if let Some(page) = self.call(Method::GET, &path, None).await? {
                v["fields"]["comment"]["comments"] = page["comments"].clone();
            }
        }
        Ok(())
    }

    /// One issue as Jira gives it, with `fields` (None: no such issue, or the account can't see it).
    pub(crate) async fn issue_raw(&self, key: &str, fields: &[String]) -> Result<Option<Value>, String> {
        self.call(Method::GET, &self.api(&format!("/issue/{key}?fields={}", fields.join(","))), None).await
    }

    /// One issue by key (None: no such issue, or the account can't see it).
    pub(crate) async fn issue(&self, key: &str) -> Result<Option<JiraIssue>, String> {
        let fields: Vec<String> = FIELDS.iter().map(|f| f.to_string()).collect();
        Ok(self.issue_raw(key, &fields).await?.and_then(|v| parse_issue(self.site(), &v)))
    }

    /// The transitions open to an issue now, each with its screen's fields.
    pub(crate) async fn transitions(&self, key: &str) -> Result<Vec<Transition>, String> {
        let path = self.api(&format!("/issue/{key}/transitions?expand=transitions.fields"));
        let v = self.found(Method::GET, &path, None).await?;
        Ok(meta::parse_transitions(&v))
    }

    /// Moves an issue through a transition, with its screen's fields in the same request.
    pub(crate) async fn do_transition(&self, key: &str, id: &str, fields: Map<String, Value>) -> Result<(), String> {
        let mut body = json!({"transition": {"id": id}});
        if !fields.is_empty() {
            body["fields"] = Value::Object(fields);
        }
        self.found(Method::POST, &self.api(&format!("/issue/{key}/transitions")), Some(&body)).await?;
        Ok(())
    }

    /// Adds a comment (text, as the deployment writes it); returns its id.
    pub(crate) async fn comment(&self, key: &str, text: &str) -> Result<String, String> {
        let body = json!({"body": self.deployment.rich_text(text)});
        let v = self.found(Method::POST, &self.api(&format!("/issue/{key}/comment")), Some(&body)).await?;
        Ok(v["id"].as_str().map(String::from).unwrap_or_else(|| v["id"].to_string()))
    }

    /// Creates an issue from `fields` (as Jira names them); returns its key.
    pub(crate) async fn create(&self, fields: Map<String, Value>) -> Result<String, String> {
        let body = json!({"fields": fields});
        let v = self.found(Method::POST, &self.api("/issue"), Some(&body)).await?;
        v["key"].as_str().map(String::from).ok_or_else(|| "Jira made the issue but did not say its key".into())
    }

    /// Sets an issue's fields.
    pub(crate) async fn edit(&self, key: &str, fields: Map<String, Value>) -> Result<(), String> {
        let body = json!({"fields": fields});
        self.found(Method::PUT, &self.api(&format!("/issue/{key}")), Some(&body)).await?;
        Ok(())
    }

    /// Assigns an issue to a user (by id), or nobody.
    pub(crate) async fn assign(&self, key: &str, user: Option<&str>) -> Result<(), String> {
        let body = match (user, self.deployment) {
            (Some(u), d) => d.user(u),
            (None, Deployment::Cloud) => json!({"accountId": null}),
            (None, Deployment::Server) => json!({"name": null}),
        };
        self.found(Method::PUT, &self.api(&format!("/issue/{key}/assignee")), Some(&body)).await?;
        Ok(())
    }

    /// The fields an issue's edit screen has.
    pub(crate) async fn editmeta(&self, key: &str) -> Result<Vec<FieldMeta>, String> {
        let v = self.found(Method::GET, &self.api(&format!("/issue/{key}/editmeta")), None).await?;
        Ok(meta::parse_fields(&v["fields"]))
    }

    /// The fields the create screen of an issue type in a project has; None when this Jira can't say (older Server
    /// has no such endpoint), or has no such type.
    pub(crate) async fn createmeta(&self, project: &str, issue_type: &str) -> Result<Option<Vec<FieldMeta>>, String> {
        let path = self.api(&format!("/issue/createmeta/{project}/issuetypes?maxResults=200"));
        let Some(types) = self.call(Method::GET, &path, None).await? else { return Ok(None) };
        let list =
            types.get("issueTypes").or(types.get("values")).and_then(Value::as_array).cloned().unwrap_or_default();
        let Some(id) = list
            .iter()
            .find(|t| t["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(issue_type.trim())))
            .and_then(|t| t["id"].as_str())
        else {
            let names: Vec<&str> = list.iter().filter_map(|t| t["name"].as_str()).collect();
            return Err(format!("{project} has no issue type {issue_type:?} (it has: {})", names.join(", ")));
        };
        let path = self.api(&format!("/issue/createmeta/{project}/issuetypes/{id}?maxResults=200"));
        let Some(v) = self.call(Method::GET, &path, None).await? else { return Ok(None) };
        let fields = v.get("fields").or(v.get("values")).cloned().unwrap_or(Value::Null);
        Ok(Some(meta::parse_fields(&fields)))
    }

    /// The Epic Name, Epic Link and Sprint fields: the connection's mappings, else found by their type.
    pub(crate) async fn special(&self) -> Result<Special, String> {
        let m = &self.mappings;
        if m.epic_name.is_some() && m.epic_link.is_some() && m.sprint.is_some() {
            return Ok(Special {
                epic_name: m.epic_name.clone(),
                epic_link: m.epic_link.clone(),
                sprint: m.sprint.clone(),
            });
        }
        let all = self.call(Method::GET, &self.api("/field"), None).await?.unwrap_or(Value::Null);
        Ok(Special {
            epic_name: m.epic_name.clone().or_else(|| meta::field_of_type(&all, meta::EPIC_NAME_TYPE)),
            epic_link: m.epic_link.clone().or_else(|| meta::field_of_type(&all, meta::EPIC_LINK_TYPE)),
            sprint: m.sprint.clone().or_else(|| meta::field_of_type(&all, meta::SPRINT_TYPE)),
        })
    }

    /// The id of the one user `who` names (`me`, a username or account id, an email or a display name).
    pub(crate) async fn user(&self, who: &str) -> Result<String, String> {
        let w = who.trim();
        if w.eq_ignore_ascii_case("me") {
            return self.myself().await;
        }
        let q: String = url_escape(w);
        let path = match self.deployment {
            Deployment::Cloud => format!("/rest/api/3/user/search?query={q}"),
            Deployment::Server => format!("/rest/api/2/user/search?username={q}"),
        };
        let v = self.call(Method::GET, &path, None).await?.unwrap_or(Value::Null);
        let users = v.as_array().cloned().unwrap_or_default();
        let ids: Vec<String> = users.iter().filter_map(|u| self.deployment.user_id(u)).collect();
        let exact = users.iter().find(|u| {
            ["accountId", "name", "key", "emailAddress", "displayName"]
                .iter()
                .any(|k| u[*k].as_str().is_some_and(|x| x.eq_ignore_ascii_case(w)))
        });
        match (exact.and_then(|u| self.deployment.user_id(u)), ids.as_slice()) {
            (Some(id), _) => Ok(id),
            (None, [one]) => Ok(one.clone()),
            (None, []) => match self.deployment {
                // A username the search can't see (it needs Browse Users) still works.
                Deployment::Server => Ok(w.to_string()),
                Deployment::Cloud => Err(format!("Jira has no user {w:?}")),
            },
            (None, many) => Err(format!("{w:?} matches {} Jira users; name one exactly", many.len())),
        }
    }

    // ---- the Agile API ----

    /// Every page of an Agile list (`values`), up to [`MAX_PAGES`] of 50.
    async fn agile_list(&self, path: &str) -> Result<Vec<Value>, String> {
        let mut out = vec![];
        let sep = if path.contains('?') { '&' } else { '?' };
        for _ in 0..MAX_PAGES {
            let p = format!("{AGILE}{path}{sep}startAt={}&maxResults=50", out.len());
            let v = self.found(Method::GET, &p, None).await?;
            let values = v["values"].as_array().cloned().unwrap_or_default();
            let n = values.len();
            out.extend(values);
            if n == 0 || v["isLast"].as_bool() != Some(false) {
                break;
            }
        }
        Ok(out)
    }

    pub(crate) async fn boards(&self, project: Option<&str>, name: Option<&str>) -> Result<Vec<Value>, String> {
        let mut q = vec![];
        if let Some(p) = project {
            q.push(format!("projectKeyOrId={}", url_escape(p)));
        }
        if let Some(n) = name {
            q.push(format!("name={}", url_escape(n)));
        }
        let path = if q.is_empty() { "/board".to_string() } else { format!("/board?{}", q.join("&")) };
        self.agile_list(&path).await
    }

    /// A board's id from its id or name.
    pub(crate) async fn board_id(&self, board: &str) -> Result<i64, String> {
        let b = board.trim();
        if let Ok(id) = b.parse::<i64>() {
            return Ok(id);
        }
        let found = self.boards(None, Some(b)).await?;
        let exact: Vec<&Value> =
            found.iter().filter(|x| x["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(b))).collect();
        match (exact.as_slice(), found.as_slice()) {
            ([one], _) => one["id"].as_i64().ok_or_else(|| "Jira gave a board without an id".into()),
            ([], [one]) => one["id"].as_i64().ok_or_else(|| "Jira gave a board without an id".into()),
            ([], []) => Err(format!("Jira has no board {b:?}")),
            _ => Err(format!("{b:?} names several boards; give its id (jira_boards lists them)")),
        }
    }

    /// A board's sprints in `states` (`active,future`).
    pub(crate) async fn sprints(&self, board: i64, states: &str) -> Result<Vec<Sprint>, String> {
        let list = self.agile_list(&format!("/board/{board}/sprint?state={}", url_escape(states))).await?;
        Ok(list.iter().filter_map(|s| meta::parse_sprint(s, &self.mappings)).collect())
    }

    /// One page of a board's issues (from `start`), with `fields`, and how many it has.
    pub(crate) async fn board_issues(
        &self,
        board: i64,
        jql: Option<&str>,
        fields: &[String],
        start: usize,
        limit: usize,
    ) -> Result<(Vec<Value>, i64), String> {
        let mut path = format!(
            "{AGILE}/board/{board}/issue?startAt={start}&maxResults={limit}&fields={}",
            url_escape(&fields.join(","))
        );
        if let Some(j) = jql.map(str::trim).filter(|j| !j.is_empty()) {
            path.push_str(&format!("&jql={}", url_escape(j)));
        }
        let v = self.found(Method::GET, &path, None).await?;
        let issues = v["issues"].as_array().cloned().unwrap_or_default();
        Ok((issues.clone(), v["total"].as_i64().unwrap_or(issues.len() as i64)))
    }

    /// Moves issues into a sprint.
    pub(crate) async fn to_sprint(&self, sprint: i64, keys: &[String]) -> Result<(), String> {
        let body = json!({"issues": keys});
        self.found(Method::POST, &format!("{AGILE}/sprint/{sprint}/issue"), Some(&body)).await?;
        Ok(())
    }

    /// Moves issues out of their sprints, to the backlog.
    pub(crate) async fn to_backlog(&self, keys: &[String]) -> Result<(), String> {
        let body = json!({"issues": keys});
        self.found(Method::POST, &format!("{AGILE}/backlog/issue"), Some(&body)).await?;
        Ok(())
    }

    /// Puts issues in an epic (or none) through the Agile API: for a Server without an Epic Link field.
    pub(crate) async fn agile_epic(&self, epic: Option<&str>, keys: &[String]) -> Result<(), String> {
        let body = json!({"issues": keys});
        let path = format!("{AGILE}/epic/{}/issue", epic.unwrap_or("none"));
        self.found(Method::POST, &path, Some(&body)).await?;
        Ok(())
    }
}

/// A query parameter's value, percent-encoded.
pub(crate) fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes_query_values() {
        assert_eq!(super::url_escape("a b,c=d&é"), "a%20b%2Cc%3Dd%26%C3%A9");
    }
}
