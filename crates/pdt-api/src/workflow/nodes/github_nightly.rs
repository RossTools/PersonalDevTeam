//! `github.nightly`: watch for a day's scheduled release (any tag pattern, PDT-294; PDT's nightlies in version 1,
//! PDT-288) and put a missing one in the Inbox until it is published. `repo` defaults to the run's project's GitHub
//! repository.

use super::forge_call::forge_call;
use super::repo_of::{RepoOf, check_repo, has_releases, repo_for_step, templated};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, repo_missing, spec,
};
use pdt_forge::{Forge, Release};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use ts_rs::TS;

/// How often [`GithubNightly`] looks again while a release is missing.
const NIGHTLY_LOOK_MS: u64 = 30 * 60_000;
/// How long it watches a missing release at most: until shortly before the next day's check takes over.
const NIGHTLY_WATCH_MS: i64 = 23 * 60 * 60_000 + 30 * 60_000;
/// Its choices (ports): start the release workflow, or leave it.
const START: &str = "start";
const IGNORE: &str = "ignore";
const MISSING_TITLE: &str = "A scheduled release is missing";

fn yes() -> bool {
    true
}

/// Version 2 (PDT-294): any tag scheme, due time and branch; version 1 knew only PDT's nightlies ([`migrate_nightly_v1`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubNightlyConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The release workflow's file (`publish.yml`): its runs are linked, and Start it runs it.
    pub workflow: String,
    /// The branch the release is built from (default `main`).
    #[serde(default)]
    pub branch: Option<String>,
    /// The scheduled release's tag: a glob where `*` is any text, `?` one character and `{date}` the day
    /// (`YYYYMMDD` or `YYYY-MM-DD`), e.g. `v*-beta.{date}*` or `snapshot-{date}`. Without `{date}`, a matching
    /// release published on the day counts.
    pub tag_pattern: String,
    /// The day whose release to look for (`2026-10-30`, e.g. `{{trigger.date}}`); today (UTC) when left out.
    #[serde(default)]
    pub day: Option<String>,
    /// When (UTC, `HH:MM`) the day's release should be out by; before it, it says nothing (default `00:00`).
    #[serde(default)]
    #[ts(optional)]
    pub due_by: Option<String>,
    /// Report it missing only if the branch has commits the newest matching release (or the latest release) lacks
    /// (default true): a scheduled workflow that publishes nothing new on a quiet day isn't missing.
    #[serde(default = "yes")]
    pub only_if_changed: bool,
    /// The Inbox item's title (default "A scheduled release is missing").
    #[serde(default)]
    #[ts(optional)]
    pub title: Option<String>,
}

/// A version 1 config (PDT-288: PDT's own nightlies, `X.Y.Z-nightly.YYYYMMDD`, due at `dueHour`, default 10) as version
/// 2, so a workflow written for it runs as before.
fn migrate_nightly_v1(config: Value) -> Result<Value, String> {
    let mut m = match config {
        Value::Object(m) => m,
        Value::Null => Default::default(),
        _ => return Err("the config is not an object".into()),
    };
    let hour = match m.remove("dueHour") {
        None | Some(Value::Null) => 10,
        Some(v) => v.as_u64().filter(|h| *h < 24).ok_or("dueHour is an hour, 0 to 23")?,
    };
    m.insert("dueBy".into(), json!(format!("{hour:02}:00")));
    m.insert("tagPattern".into(), json!("*-nightly.{date}*"));
    m.entry("title").or_insert_with(|| json!("Tonight's nightly didn't run"));
    m.insert("onlyIfChanged".into(), json!(true));
    Ok(Value::Object(m))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubNightlyOutput {
    /// The day looked at (`20261030`).
    pub day: u64,
    /// With `published`: the day's release.
    pub tag: Option<String>,
    pub url: Option<String>,
    /// The release workflow's runs on GitHub.
    pub runs_url: String,
}

/// What the watch read: the day's release, if published, and whether the branch has commits the newest matching
/// release and the latest release lack (asked only when it matters).
struct ScheduledRead {
    published: Option<Release>,
    changed: bool,
    /// What the branch was compared with, for the item's text.
    base: Option<String>,
    ahead: u64,
}

struct Look {
    repo: String,
    branch: String,
    pattern: pdt_core::release::TagPattern,
    day: u64,
    /// The due time has passed: whether the branch changed matters.
    due: bool,
    only_if_changed: bool,
}

async fn read_scheduled(forge: &Arc<dyn Forge>, look: Look) -> Result<ScheduledRead, String> {
    use pdt_core::release::rfc3339_day;
    has_releases(forge)?;
    forge_call(forge, move |f| {
        let Look { repo: r, branch: b, pattern, day, due, only_if_changed } = look;
        let releases = f.recent_releases(&r)?;
        // Each matching release with its day: `{date}`'s, else the day it was published.
        let matching: Vec<(&Release, Option<u64>)> = releases
            .iter()
            .filter_map(|x| {
                let date = pattern.matches(&x.tag)?;
                Some((x, date.or_else(|| x.published_at.as_deref().and_then(rfc3339_day))))
            })
            .collect();
        let published = matching.iter().find(|(_, d)| d.is_some_and(|d| d >= day)).map(|(x, _)| (*x).clone());
        let mut read = ScheduledRead { published, changed: true, base: None, ahead: 0 };
        if read.published.is_some() || !due || !only_if_changed {
            return Ok(read);
        }
        let mut bases: Vec<String> = matching.first().map(|(x, _)| x.tag.clone()).into_iter().collect();
        if let Some(latest) = f.release(&r, None)?.filter(|l| !bases.contains(&l.tag)) {
            bases.push(latest.tag);
        }
        // Nothing released yet: anything on the branch is new.
        for base in &bases {
            match f.commits_ahead(&r, base, &b)? {
                Some(0) => {
                    read.changed = false;
                    break;
                }
                Some(n) if read.base.is_none() => (read.base, read.ahead) = (Some(base.clone()), n),
                _ => {}
            }
        }
        Ok(read)
    })
    .await
}

/// `20261030` as `2026-10-30`.
fn iso_day(day: u64) -> String {
    format!("{}-{:02}-{:02}", day / 10_000, day / 100 % 100, day % 100)
}

pub struct GithubNightly;

impl NodeKind for GithubNightly {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubNightlyConfig, GithubNightlyOutput>(
            "github.nightly",
            2,
            Category::Action,
            "Watch for a scheduled release",
            "Looks for a day's scheduled release: a published release (pre-releases included) whose tag matches \
             `tagPattern` (`*` any text, `?` one character, `{date}` the day as `YYYYMMDD` or `YYYY-MM-DD`; without \
             `{date}`, one published that day), the day's or a later one. `published` when there is one; before \
             `dueBy` (UTC) `early`; with `onlyIfChanged` (the default), `unchanged` when the branch has nothing the \
             newest matching release or the latest release lacks. Otherwise it is missing: one Inbox item says so \
             (once per workflow and day, else `already`), with Start it (`start`, e.g. to a step that runs the \
             workflow) and Ignore (`ignore`), and it looks again every half hour: the item closes by itself once the \
             release is published (`published`), or after about a day (`expired`).",
            &[
                ("published", "Published"),
                ("early", "Not due yet"),
                ("unchanged", "Branch unchanged"),
                (START, "Start it"),
                (IGNORE, "Ignore"),
                ("already", "Already in the Inbox"),
                ("expired", "Never came"),
            ],
            false,
        )
    }

    fn migrate(&self, from: u32, config: Value) -> Result<Value, String> {
        match from {
            1 => migrate_nightly_v1(config),
            2 => Ok(config),
            v => Err(format!("no migration from version {v} to 2")),
        }
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubNightlyConfig = parse(config)?;
        if c.workflow.trim().is_empty() {
            return Err("workflow is empty: name its file (publish.yml)".into());
        }
        if !templated(&c.tag_pattern) {
            pdt_core::release::TagPattern::parse(&c.tag_pattern)?;
        }
        if let Some(d) = nonblank(&c.day).filter(|d| !templated(d))
            && pdt_core::release::parse_day(d).is_none()
        {
            return Err(format!("day {d:?} is not a date (2026-10-30)"));
        }
        if let Some(t) = nonblank(&c.due_by).filter(|t| !templated(t))
            && pdt_core::release::parse_time_of_day(t).is_none()
        {
            return Err(format!("dueBy {t:?} is not a time of day (HH:MM, UTC)"));
        }
        check_repo(&c.repo)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind};
            use pdt_core::release::{
                ScheduledWatch, TagPattern, parse_day, parse_time_of_day, scheduled_watch, utc_day,
            };
            let c: GithubNightlyConfig = parse(&input)?;
            let RepoOf { repo, forge, .. } = repo_for_step(cx, &c.repo).await?;
            let workflow = c.workflow.trim().to_string();
            if workflow.is_empty() {
                return Err("workflow is empty: name its file (publish.yml)".into());
            }
            let pattern_text = c.tag_pattern.trim().to_string();
            let pattern = TagPattern::parse(&pattern_text)?;
            let branch = nonblank(&c.branch).unwrap_or("main").to_string();
            let now = crate::routes::now_ms();
            // A look after the first keeps the first's day and start (its saved output).
            let saved = |k: &str| cx.step.output.get(k).and_then(Value::as_i64);
            let day = match saved("day") {
                Some(d) => d as u64,
                None => match nonblank(&c.day) {
                    Some(d) => parse_day(d).ok_or_else(|| format!("day {d:?} is not a date"))?,
                    None => utc_day(now),
                },
            };
            let since = saved("since").unwrap_or(now);
            let due_by = match nonblank(&c.due_by) {
                Some(t) => parse_time_of_day(t).ok_or_else(|| format!("dueBy {t:?} is not a time of day (HH:MM)"))?,
                None => 0,
            };
            let due_text = format!("{:02}:{:02}", due_by / 60, due_by % 60);
            let runs_url = format!("https://github.com/{repo}/actions/workflows/{workflow}");
            let out = |tag: Option<&Release>| GithubNightlyOutput {
                day,
                tag: tag.map(|r| r.tag.clone()),
                url: tag.map(|r| r.url.clone()),
                runs_url: runs_url.clone(),
            };
            if cx.dry_run {
                return Ok(Outcome::would(
                    "unchanged",
                    format!(
                        "look for {repo}'s release matching {pattern_text} for {day}, due at \
                                             {due_text} UTC"
                    ),
                ));
            }
            let title = c.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| MISSING_TITLE.into());
            // It waits for the user's choice, and (`retry`) runs again at the next look: the item stays open while
            // the step does, and closes when it leaves by any port.
            let look_again = || {
                let next = now + i64::try_from(NIGHTLY_LOOK_MS).unwrap_or(i64::MAX / 2);
                let mut progress = serde_json::to_value(out(None)).unwrap_or_default();
                progress["since"] = json!(since);
                Outcome::Wait {
                    wait: json!({"retry": true, "answer": {"question": title, "choices": [START, IGNORE]}}),
                    until: Some(next.min(since + NIGHTLY_WATCH_MS)),
                    timeout_port: None,
                    output: progress,
                }
            };
            let due = now >= pdt_core::release::day_time_ms(day, due_by);
            let look = Look {
                repo: repo.clone(),
                branch: branch.clone(),
                pattern,
                day,
                due,
                only_if_changed: c.only_if_changed,
            };
            let read = match read_scheduled(&forge, look).await {
                Ok(r) => r,
                // While it watches (its item open), a failed look waits for the next one: each look is one of the
                // step's attempts, so the node's retries would soon be used up.
                Err(e) if saved("since").is_some() => {
                    tracing::warn!(error = %e, "could not look for the day's scheduled release; looking again later");
                    return Ok(look_again());
                }
                Err(e) => return Err(e),
            };
            let quiet = |port: &str| {
                let mut o = serde_json::to_value(out(None)).unwrap_or_default();
                o["quiet"] = json!(true);
                Outcome::Next { port: port.into(), output: o }
            };
            match scheduled_watch(now, day, due_by, read.published.is_some(), read.changed) {
                ScheduledWatch::Published => return Ok(Outcome::next("published", out(read.published.as_ref()))),
                ScheduledWatch::Early => return Ok(quiet("early")),
                ScheduledWatch::Unchanged => return Ok(quiet("unchanged")),
                ScheduledWatch::Missing => {}
            }
            if now - since >= NIGHTLY_WATCH_MS {
                return Ok(quiet("expired"));
            }
            let store = &cx.st.store;
            let open = store.open_items_for_step(&cx.step.id).await.map_err(crate::workflow::err)?;
            if open.is_empty() {
                let changed = match (&read.base, read.ahead) {
                    _ if !c.only_if_changed => String::new(),
                    (Some(b), n) if n > 0 => {
                        format!(", though {branch} has {n} commit{} since {b}", if n == 1 { "" } else { "s" })
                    }
                    _ => format!(", though {branch} has changed since the last release"),
                };
                let text = format!(
                    "No release matching `{pattern_text}` for {} was published by {due_text} UTC{changed}. GitHub's \
                     scheduled runs are best-effort and sometimes never start.\n\nSee [the release workflow's \
                     runs]({runs_url}). **Start it** goes on with the workflow's next step (usually running \
                     `{workflow}` on {branch} as your GitHub login); if your login can't start workflows, start it \
                     there (Run workflow). This item goes away by itself once a release for the day is published.",
                    iso_day(day)
                );
                let body = json!({"question": title, "text": text, "url": runs_url, "workflow": cx.workflow.name,
                                  "node": cx.node_title()});
                let item = InboxItem {
                    created_at: now,
                    ..cx.inbox_item(InboxKind::Gate, InboxGroup::Gate, &title, body).choices(vec![
                        InboxChoice::new(START, "Start it", true),
                        InboxChoice::new(IGNORE, "Ignore", false),
                    ])
                };
                // Once per workflow and day: a second run the same day (a catch-up) leaves it to the first. (The key
                // keeps version 1's name, so an upgrade on the day adds no second item.)
                let key = format!("nightly-missing:{}:{day}", cx.workflow.id);
                match store.add_inbox_item_once(&item, &key).await.map_err(crate::workflow::err)? {
                    Some(_) => crate::notifications::kick(cx.st),
                    None => return Ok(quiet("already")),
                }
            }
            Ok(look_again())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let nightly = |v: Value| GithubNightly.check(&v);
        let w = |extra: Value| {
            let mut c = json!({"workflow": "publish.yml", "tagPattern": "v*-beta.{date}"});
            c.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            c
        };
        assert!(nightly(w(json!({"day": "{{trigger.date}}", "dueBy": "10:30"}))).is_ok());
        assert!(nightly(w(json!({"day": "2026-10-30", "onlyIfChanged": false}))).is_ok());
        assert!(nightly(w(json!({"tagPattern": "{{params.tag}}", "dueBy": "{{params.hour}}:00"}))).is_ok());
        assert!(nightly(w(json!({"day": "Friday"}))).unwrap_err().contains("not a date"));
        assert!(nightly(w(json!({"dueBy": "24:00"}))).unwrap_err().contains("time of day"));
        assert!(nightly(w(json!({"tagPattern": " "}))).unwrap_err().contains("empty"));
        assert!(nightly(w(json!({"tagPattern": "{date}{date}"}))).unwrap_err().contains("twice"));
        assert!(nightly(json!({"workflow": "publish.yml"})).is_err(), "tagPattern is needed");
        assert!(nightly(w(json!({"workflow": " "}))).is_err());
        assert!(nightly(w(json!({"dueHour": 10}))).is_err(), "version 1's field");
    }

    #[test]
    fn a_version_1_nightly_config_migrates() {
        let old = json!({"repo": "a/b", "workflow": "release.yml", "branch": "main", "day": "{{trigger.date}}",
                         "dueHour": 7});
        let new = GithubNightly.migrate(1, old).unwrap();
        assert_eq!(
            new,
            json!({"repo": "a/b", "workflow": "release.yml", "branch": "main", "day": "{{trigger.date}}",
                   "dueBy": "07:00", "tagPattern": "*-nightly.{date}*", "onlyIfChanged": true,
                   "title": "Tonight's nightly didn't run"})
        );
        GithubNightly.check(&new).unwrap();
        let pattern = pdt_core::release::TagPattern::parse(new["tagPattern"].as_str().unwrap()).unwrap();
        for (tag, day) in [("v0.2.1-nightly.20261030", Some(20261030)), ("0.2.1-nightly.20261030.2", Some(20261030))] {
            assert_eq!(pattern.matches(tag), Some(day), "{tag}");
        }
        assert_eq!(pattern.matches("v0.2.0"), None);
        assert_eq!(pattern.matches("v0.2.1-exp.3"), None);

        // Its defaults: due at 10:00, PDT's title; one the user gave stays.
        let new = GithubNightly.migrate(1, json!({"workflow": "release.yml", "title": "Mine"})).unwrap();
        assert_eq!((new["dueBy"].as_str(), new["title"].as_str()), (Some("10:00"), Some("Mine")));
        assert!(GithubNightly.migrate(1, json!({"workflow": "r.yml", "dueHour": 30})).is_err());
        assert!(GithubNightly.migrate(3, json!({})).is_err());
        let current = json!({"workflow": "p.yml", "tagPattern": "v*"});
        assert_eq!(GithubNightly.migrate(2, current.clone()).unwrap(), current);
    }
}
