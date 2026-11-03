//! `storage.clean` and `storage.measure` (PDT-370): what the built-in Clean up storage does hourly and when a disk runs
//! low. `crate::storage` measures and cleans; the storage loop still measures on its own.

use crate::routes::now_ms;
use crate::storage::{self, Finished};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, parse, parses, spec};
use pdt_core::storage::DiskLevel;
use pdt_core::ticket::TicketStage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// Which worktrees `storage.clean` takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CleanWhich {
    /// Those whose project's cleanup delays (its Worktrees settings) have passed: cleaned or removed as they say.
    #[default]
    Due,
    /// Every finished ticket's (Done or Won't do), now, by `stages` and `olderThanDays`.
    Finished,
}

/// What `storage.clean` does to a finished ticket's worktree (`which: finished`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CleanWhat {
    /// Its build output only.
    #[default]
    Build,
    /// The whole worktree, when nothing would be lost: never with uncommitted changes, and its branch stays.
    Worktree,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct StorageCleanConfig {
    #[serde(default)]
    #[ts(as = "Option<CleanWhich>", optional)]
    pub which: CleanWhich,
    /// `which: finished`: the tickets' stages (`done`, `wont_do`); empty: both.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<TicketStage>>", optional)]
    pub stages: Vec<TicketStage>,
    /// `which: finished`: only tickets closed at least this many days ago (0: any).
    #[serde(default)]
    #[ts(as = "Option<u32>", optional)]
    pub older_than_days: u32,
    #[serde(default)]
    #[ts(as = "Option<CleanWhat>", optional)]
    pub what: CleanWhat,
    /// One project's worktrees only (its id); empty: every project's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project: Option<String>,
}

impl StorageCleanConfig {
    fn check(&self) -> Result<(), String> {
        if let Some(s) = self.stages.iter().find(|s| !s.is_closed()) {
            return Err(format!("stages are finished ones (done, wont_do), not {}", s.label()));
        }
        if self.older_than_days > pdt_core::storage::MAX_CLEANUP_DAYS {
            return Err(format!("olderThanDays is at most {}", pdt_core::storage::MAX_CLEANUP_DAYS));
        }
        Ok(())
    }
}

pub struct StorageClean;

impl NodeKind for StorageClean {
    fn spec(&self) -> KindSpec {
        spec::<StorageCleanConfig, Value>(
            "storage.clean",
            1,
            Category::Action,
            "Clean up worktrees",
            "Frees disk space in finished tickets' worktrees. `which: due` (the default) does what each project's \
             Worktrees settings say once their delays have passed. `which: finished` takes every Done or Won't do \
             ticket's worktree now (`stages`, `olderThanDays`) and removes its build output, or with `what: \
             worktree` the whole worktree (its branch stays). Worktrees in use or with uncommitted changes are left \
             alone. Its output: `cleaned`, `removed`, `freed` (bytes), `skipped`.",
            &[("out", "Then")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parse::<StorageCleanConfig>(config)?.check()
    }

    fn default_timeout_ms(&self) -> u64 {
        30 * 60 * 1000
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: StorageCleanConfig = parse(&input)?;
            c.check()?;
            if cx.dry_run {
                return Ok(Outcome::would("out", "clean up finished tickets' worktrees"));
            }
            let r = match c.which {
                CleanWhich::Due => storage::auto_cleanup(cx.st).await,
                CleanWhich::Finished => {
                    let mut f = Finished {
                        older_than_days: c.older_than_days,
                        project: c.project.filter(|p| !p.trim().is_empty()),
                        remove: c.what == CleanWhat::Worktree,
                        ..Default::default()
                    };
                    if !c.stages.is_empty() {
                        f.stages = c.stages;
                    }
                    storage::clean_finished_now(cx.st, &f, now_ms()).await.map_err(|e| e.message().to_string())?
                }
            };
            if r.cleaned + r.removed > 0 {
                storage::spawn_measure(cx.st, false);
            }
            Ok(Outcome::next("out", r))
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct StorageMeasureConfig {
    /// Below this many GB free a disk is low (0: never); empty: Clean up storage's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub low_gb: Option<u32>,
    /// Below this many GB free it is critical (0: never); empty: Clean up storage's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub critical_gb: Option<u32>,
}

pub struct StorageMeasure;

impl NodeKind for StorageMeasure {
    fn spec(&self) -> KindSpec {
        spec::<StorageMeasureConfig, Value>(
            "storage.measure",
            1,
            Category::Condition,
            "Check disk space",
            "Reads the free space of each disk PDT uses (its data folder's and the projects') and goes on by the \
             fullest one's level against `lowGb` and `criticalGb`. Its output: `level`, `free`, `total` (bytes), \
             `freeText` (\"3.0 GB\"), `folders`, and `reclaimable`: the build output in finished tickets' worktrees.",
            &[("ok", "Enough space"), ("low", "Low"), ("critical", "Almost full")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        parses::<StorageMeasureConfig>(config)
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: StorageMeasureConfig = parse(&input)?;
            let (low, critical) = match (c.low_gb, c.critical_gb) {
                (Some(l), Some(cr)) => (l, cr),
                (l, cr) => {
                    let (dl, dc) = storage::thresholds(cx.st).await;
                    (l.unwrap_or(dl), cr.unwrap_or(dc))
                }
            };
            let disks = storage::disks_at(cx.st, low, critical).await;
            let Some(worst) = disks.iter().max_by_key(|d| (d.level, std::cmp::Reverse(d.space.free))) else {
                return Ok(Outcome::next("ok", json!({"level": "ok", "reason": "no disk could be read"})));
            };
            let out = json!({
                "level": worst.level.as_str(),
                "free": worst.space.free,
                "total": worst.space.total,
                "freeText": pdt_core::inbox::human_bytes(worst.space.free),
                "folders": worst.folders,
                "reclaimable": storage::finished_build_bytes(cx.st).await,
                "lowGb": low,
                "criticalGb": critical,
            });
            let port = match worst.level {
                DiskLevel::Ok => "ok",
                DiskLevel::Low => "low",
                DiskLevel::Critical => "critical",
            };
            Ok(Outcome::next(port, out))
        })
    }
}
