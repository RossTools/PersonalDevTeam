//! `host.wait_idle`, `host.install_update` and `host.check_health` (PDT-369): what the built-in Install updates does
//! with an update on offer, and with the version it started on afterwards. `update.rs` does the work.

use crate::error::codes;
use crate::routes::now_ms;
use crate::update;
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, parses, spec};
use pdt_core::host::{InstallWindow, UpdatePolicy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// How often a waiting step looks again, by default.
const CHECK_EVERY_MS: u64 = 60_000;
/// How long pdtd runs before the health check looks, by default: its loops make their first passes.
const SETTLE_MS: u64 = 3 * 60_000;

fn default_check_every() -> u64 {
    CHECK_EVERY_MS
}

fn default_settle() -> u64 {
    SETTLE_MS
}

fn default_true() -> bool {
    true
}

/// When the step began waiting (kept in its output across its looks), else now.
fn since(cx: &StepCx<'_>, now: i64) -> i64 {
    cx.step.output.get("since").and_then(Value::as_i64).unwrap_or(now)
}

/// Looks again in `every_ms`, saying why it waits (`since`: what the next look reads back).
fn wait_again(since: Option<i64>, every_ms: u64, why: &str) -> Outcome {
    let every = i64::try_from(every_ms.max(10)).unwrap_or(i64::MAX / 2);
    Outcome::Wait {
        wait: json!({"retry": true, (crate::workflow::engine::POLL): true, "waiting": why}),
        until: Some(now_ms() + every),
        timeout_port: None,
        output: json!({"since": since, "waiting": why}),
    }
}

/// What the host's update setting says about installing by itself, when it doesn't.
fn not_automatic(p: UpdatePolicy) -> Option<&'static str> {
    match p {
        UpdatePolicy::AutoWhenIdle => None,
        UpdatePolicy::Notify => Some("updates are set to notify only (Settings → Updates)"),
        UpdatePolicy::Manual => Some("updates are set to manual (Settings → Updates)"),
    }
}

/// Why `version` is no longer to install: it is installed, or another one is on offer. None while it is the one.
fn gone(cx: &StepCx<'_>, version: &str) -> Option<String> {
    let svc = &cx.st.update;
    if update::installed(svc, version) {
        return Some(format!("v{version} is installed (PDT runs v{})", update::current(svc)));
    }
    // Right after a restart nothing is on offer until the first check: that doesn't end the wait.
    match update::on_offer(svc) {
        Some(o) if o.version != version => Some(format!("v{} is on offer now", o.version)),
        _ => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct WindowConfig {
    pub on: bool,
    /// `HH:MM`, this computer's time.
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct WaitIdleConfig {
    /// An update's version (`{{trigger.version}}`): it also waits while the host's updates aren't set to install by
    /// themselves, and leaves by `skipped` once that version is installed or another is on offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub update: Option<String>,
    /// Only between these times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub window: Option<WindowConfig>,
    /// Still busy after this long: leave by `timeout`. 0: wait as long as it takes.
    #[serde(default)]
    #[ts(type = "number")]
    pub max_wait_ms: u64,
    /// How often it looks again.
    #[serde(default = "default_check_every")]
    #[ts(as = "Option<f64>", optional)]
    pub check_every_ms: u64,
}

impl WaitIdleConfig {
    fn window(&self) -> Result<Option<InstallWindow>, String> {
        match &self.window {
            Some(w) if w.on => InstallWindow::parse(&w.from, &w.to).map(Some),
            _ => Ok(None),
        }
    }
}

/// Whether a `host.wait_idle` config fits: its window's times are times of day (on or off, unless a reference).
pub fn check_wait(config: &Value) -> Result<(), String> {
    let c: WaitIdleConfig = parse(config)?;
    match &c.window {
        Some(w) if !w.from.contains("{{") && !w.to.contains("{{") => InstallWindow::parse(&w.from, &w.to).map(|_| ()),
        _ => Ok(()),
    }
}

pub struct WaitIdle;

impl NodeKind for WaitIdle {
    fn spec(&self) -> KindSpec {
        spec::<WaitIdleConfig, Value>(
            "host.wait_idle",
            1,
            Category::Wait,
            "Wait until PDT is idle",
            "Waits until no agent is working or waiting on background tasks and no run is setting up (what makes PDT \
             busy for an update), inside the window if one is set. `timeout` when it is still busy after `maxWaitMs`. \
             With `update`, it also waits while updates aren't set to install by themselves, and `skipped` once that \
             version is installed or another is on offer.",
            &[("idle", "Idle"), ("timeout", "Still busy"), ("skipped", "Not this update")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        check_wait(config)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: WaitIdleConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::would("idle", "wait until PDT is idle"));
            }
            let now = now_ms();
            // When PDT was first found busy inside the window: only that time counts towards `maxWaitMs`.
            let busy_since = cx.step.output.get("since").and_then(Value::as_i64);
            if let Some(v) = c.update.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                if let Some(why) = gone(cx, v) {
                    return Ok(Outcome::next("skipped", json!({"reason": why})));
                }
                if let Some(why) = not_automatic(update::refresh_settings(cx.st).await) {
                    return Ok(Outcome::next("skipped", json!({"reason": why})));
                }
            }
            let in_window = c.window()?.is_none_or(|w| w.contains(crate::notifications::minute_now()));
            let busy = if in_window { update::busy_reasons(cx.st).await } else { Vec::new() };
            Ok(match look(busy_since, now, in_window, !busy.is_empty(), c.max_wait_ms) {
                Look::Idle => Outcome::next("idle", json!({})),
                Look::Timeout { waited } => Outcome::next("timeout", json!({"busy": busy, "waitedMs": waited})),
                Look::Wait { since } if !in_window => {
                    let w = c.window.as_ref();
                    let why = format!("outside the window {}–{}", w.map_or("", |w| &w.from), w.map_or("", |w| &w.to));
                    wait_again(since, c.check_every_ms, &why)
                }
                Look::Wait { since } => wait_again(since, c.check_every_ms, &format!("busy: {}", busy.join("; "))),
            })
        })
    }
}

/// What one look of `host.wait_idle` comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Idle,
    /// Busy inside the window for `waited` ms, at least `maxWaitMs`.
    Timeout {
        waited: i64,
    },
    /// Look again; `since` is when PDT was first found busy inside the window (None outside it: the clock starts
    /// again at the next look inside).
    Wait {
        since: Option<i64>,
    },
}

/// `since`: when an earlier look first found PDT busy inside the window. Only time busy inside the window counts
/// towards `max_wait_ms`: a window that opens on a busy PDT waits for its agents first.
fn look(since: Option<i64>, now: i64, in_window: bool, busy: bool, max_wait_ms: u64) -> Look {
    if !in_window {
        return Look::Wait { since: None };
    }
    if !busy {
        return Look::Idle;
    }
    let since = since.unwrap_or(now);
    let waited = now - since;
    if max_wait_ms > 0 && waited >= i64::try_from(max_wait_ms).unwrap_or(i64::MAX) {
        return Look::Timeout { waited };
    }
    Look::Wait { since: Some(since) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct InstallUpdateConfig {
    /// The version to install (`{{trigger.version}}`); `skipped` when another one is on offer. Empty: the one on
    /// offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// Installed by PDT itself: only while updates are set to install when idle, and never a version that failed
    /// before (install that one from Settings → Updates).
    #[serde(default = "default_true")]
    #[ts(as = "Option<bool>", optional)]
    pub automatic: bool,
    /// Install even while PDT is busy (sessions resume afterwards).
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub force: bool,
}

pub struct InstallUpdate;

impl NodeKind for InstallUpdate {
    fn spec(&self) -> KindSpec {
        spec::<InstallUpdateConfig, Value>(
            "host.install_update",
            1,
            Category::Action,
            "Install the update",
            "Downloads and verifies the update on offer and hands it to the installer: PDT restarts on it (and \
             `update.installed` follows, or `update.failed`). `busy` when PDT got busy (unless `force`); `skipped` when \
             it isn't on offer, is installed, or (`automatic`) updates aren't set to install by themselves or this \
             version failed before.",
            &[("installing", "Installing"), ("busy", "PDT is busy"), ("skipped", "Not installed")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<InstallUpdateConfig>(config)
    }

    fn default_timeout_ms(&self) -> u64 {
        30 * 60 * 1000
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: InstallUpdateConfig = parse(&input)?;
            let skip = |why: String| Ok(Outcome::next("skipped", json!({"reason": why})));
            let version = c.version.as_deref().map(str::trim).filter(|v| !v.is_empty());
            // After a restart this finds its version installed (or rolled back, and skipped by the check below).
            if let Some(why) = version.and_then(|v| gone(cx, v)) {
                return skip(why);
            }
            let Some(offer) = update::on_offer(&cx.st.update) else { return skip("no update is on offer".into()) };
            if c.automatic {
                if let Some(why) = not_automatic(update::refresh_settings(cx.st).await) {
                    return skip(why.into());
                }
                if update::skipped(&cx.st.data_dir, &offer.version) {
                    return skip(format!(
                        "the last attempt at v{} failed: install it from Settings → Updates",
                        offer.version
                    ));
                }
            }
            if cx.dry_run {
                return Ok(Outcome::would("installing", format!("install v{}", offer.version)));
            }
            match update::install(cx.st, c.force, c.automatic).await {
                Ok(_) => Ok(Outcome::next("installing", json!({"version": offer.version}))),
                Err(e) if e.code() == codes::HOST_BUSY => Ok(Outcome::next("busy", json!({"busy": e.message()}))),
                Err(e) if matches!(e.code(), "no_update" | "updates_disabled") => skip(e.message().into()),
                Err(e) => Err(e.message().to_string()),
            }
        })
    }

    /// The download is kept, and a version already installed is `skipped`.
    fn rerun_safe(&self, _input: &Value) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct CheckHealthConfig {
    /// The version PDT should run (`{{trigger.version}}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// How long to let PDT run first, so its loops make their first passes.
    #[serde(default = "default_settle")]
    #[ts(as = "Option<f64>", optional)]
    pub settle_ms: u64,
}

pub struct CheckHealth;

impl NodeKind for CheckHealth {
    fn spec(&self) -> KindSpec {
        spec::<CheckHealthConfig, Value>(
            "host.check_health",
            1,
            Category::Wait,
            "Check PDT is healthy",
            "After `settleMs`, checks what PDT's health check reports: it runs `version`, and none of its background \
             loops stopped or panicked. `unhealthy` with `problems` and `error` (them in one line) when not.",
            &[("healthy", "Healthy"), ("unhealthy", "Unhealthy")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<CheckHealthConfig>(config)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: CheckHealthConfig = parse(&input)?;
            if cx.dry_run {
                return Ok(Outcome::would("healthy", "check PDT's health"));
            }
            let now = now_ms();
            let since = since(cx, now);
            let settle = i64::try_from(c.settle_ms).unwrap_or(i64::MAX / 2);
            if now - since < settle {
                return Ok(Outcome::Wait {
                    wait: json!({"retry": true, (crate::workflow::engine::POLL): true, "waiting": "letting PDT settle"}),
                    until: Some(since + settle),
                    timeout_port: None,
                    output: json!({"since": since}),
                });
            }
            let problems = problems(cx, c.version.as_deref(), now);
            if problems.is_empty() {
                let version = update::current(&cx.st.update);
                return Ok(Outcome::next("healthy", json!({"version": version})));
            }
            Ok(Outcome::next("unhealthy", json!({"problems": problems, "error": problems.join("; ")})))
        })
    }
}

/// What `/api/health` shows wrong: another version, a loop that stopped or panicked.
fn problems(cx: &StepCx<'_>, version: Option<&str>, now: i64) -> Vec<String> {
    let mut out = Vec::new();
    let current = update::current(&cx.st.update);
    if let Some(v) = version.map(str::trim).filter(|v| !v.is_empty() && *v != current) {
        out.push(format!("PDT runs v{current}, not v{v}"));
    }
    for l in cx.st.loops.health(now) {
        if l.stale {
            out.push(format!("its {} loop stopped", l.name));
        }
        if l.panics > 0 {
            out.push(format!("its {} loop panicked {} time{}", l.name, l.panics, if l.panics == 1 { "" } else { "s" }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 60 * 60 * 1000;

    /// Install anyway after 2 h with a window: an update found by day waits outside the window without the clock
    /// running, and the window opening on a busy PDT starts it, so its agents get the 2 h.
    #[test]
    fn only_time_busy_inside_the_window_counts() {
        let max = 2 * 3_600_000;
        // Outside the window: no clock, whatever came before.
        assert_eq!(look(Some(0), 12 * HOUR, false, true, max), Look::Wait { since: None });
        // The window opens on a busy PDT: the clock starts now, not at the update's discovery.
        assert_eq!(look(None, 14 * HOUR, true, true, max), Look::Wait { since: Some(14 * HOUR) });
        assert_eq!(look(Some(14 * HOUR), 15 * HOUR, true, true, max), Look::Wait { since: Some(14 * HOUR) });
        assert_eq!(look(Some(14 * HOUR), 16 * HOUR, true, true, max), Look::Timeout { waited: 2 * HOUR });
        // Idle inside the window installs; no limit waits for the agents however long.
        assert_eq!(look(Some(0), 20 * HOUR, true, false, max), Look::Idle);
        assert_eq!(look(Some(0), 100 * HOUR, true, true, 0), Look::Wait { since: Some(0) });
    }
}
