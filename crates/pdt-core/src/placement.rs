//! Placement: which instance an attempt runs on. The ticket's Runs on override, then the project's label rules
//! (`windows` → `win-box`), then the repository's `[[placement]]` requirements matched against the joined instances'
//! facts, then the owner itself. Pure.

use crate::link::InstanceFacts;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// An instance a session or worktree runs on, as the owner shows it: the remote instance's id and its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstanceRef {
    pub id: String,
    pub name: String,
    /// When the owner last heard from it (None: never yet, or it left the project): the chip says it's offline when
    /// that is a while ago.
    #[serde(default)]
    #[ts(type = "number | null")]
    pub last_seen_at: Option<i64>,
}

/// A project's rule on the owner: tickets with `label` run on the instance `instance` (its id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LabelRule {
    pub label: String,
    pub instance: String,
}

/// A project's placement settings on the owner (Project Settings → Remote instances).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct PlacementSettings {
    pub rules: Vec<LabelRule>,
}

impl PlacementSettings {
    /// Trimmed and lower-cased labels, without blank or repeated ones (the first rule for a label wins).
    pub fn validate(self) -> Result<Self, String> {
        let mut rules: Vec<LabelRule> = Vec::new();
        for r in self.rules {
            let label = r.label.trim().to_lowercase();
            let instance = r.instance.trim().to_string();
            if label.is_empty() || instance.is_empty() {
                return Err("a placement rule needs a label and an instance".into());
            }
            if rules.iter().any(|x| x.label == label) {
                return Err(format!("the label {label} has two placement rules"));
            }
            rules.push(LabelRule { label, instance });
        }
        Ok(Self { rules })
    }
}

/// A `[[placement]]` rule from the repository's `.pdt/project.toml`: tickets with one of `labels` need an instance
/// with `os` and every one of `tools`. It never names an instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepoRule {
    pub labels: Vec<String>,
    pub os: Option<String>,
    pub tools: Vec<String>,
}

impl RepoRule {
    /// Whether an instance with `facts` meets the rule's needs.
    pub fn met_by(&self, facts: &InstanceFacts) -> bool {
        self.os.as_deref().is_none_or(|os| same_os(os, &facts.os))
            && self.tools.iter().all(|t| facts.tools.iter().any(|f| f.name.eq_ignore_ascii_case(t)))
    }
}

/// `windows`, `macos` (or `mac`, `darwin`) and `linux` name the same OS as Rust's `std::env::consts::OS`.
fn same_os(wanted: &str, reported: &str) -> bool {
    let norm = |s: &str| match s.trim().to_lowercase().as_str() {
        "mac" | "darwin" | "osx" => "macos".to_string(),
        "win" | "win32" => "windows".to_string(),
        o => o.to_string(),
    };
    norm(wanted) == norm(reported)
}

/// A joined instance as placement sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub facts: Option<&'a InstanceFacts>,
    /// Heard from lately and not paused.
    pub available: bool,
    /// Paused on the remote instance (when not available: paused rather than offline).
    pub paused: bool,
}

/// Where an attempt goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    /// The owner itself.
    Owner,
    /// A remote instance, by id, and why.
    Remote { id: String, why: Why },
}

/// Why an attempt was placed on a remote instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    Override,
    Label(String),
    Needs(String),
}

/// Why it can't be placed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    #[error("no instance called {0} has joined this project")]
    UnknownInstance(String),
    /// The instance it goes on (`id`) is offline, or paused there: the ticket waits for it.
    #[error("{name} is {}", if *paused { "paused" } else { "offline" })]
    Unavailable { id: String, name: String, paused: bool },
    /// No joined instance (nor the owner) meets the repository's needs for `label`.
    #[error("no instance can run this (needs {needs})")]
    NoMatch { label: String, needs: String },
}

/// The override names the owner itself.
pub const OWNER: &str = "owner";

/// The instance a ticket with `labels` runs on. `runs_on` is the Start dialog's choice: an instance's id or name, or
/// [`OWNER`]. The owner's own facts (`own`) count for a repository rule: a rule the owner meets keeps the work home.
pub fn place(
    runs_on: Option<&str>,
    labels: &[String],
    rules: &[LabelRule],
    repo: &[RepoRule],
    instances: &[Candidate<'_>],
    own: Option<&InstanceFacts>,
) -> Result<Placed, PlacementError> {
    let find = |key: &str| instances.iter().find(|c| c.id == key || c.name.eq_ignore_ascii_case(key));
    let pick = |c: &Candidate<'_>, why: Why| {
        if c.available {
            Ok(Placed::Remote { id: c.id.to_string(), why })
        } else {
            Err(PlacementError::Unavailable { id: c.id.to_string(), name: c.name.to_string(), paused: c.paused })
        }
    };
    if let Some(r) = runs_on.map(str::trim).filter(|r| !r.is_empty()) {
        if r.eq_ignore_ascii_case(OWNER) {
            return Ok(Placed::Owner);
        }
        let c = find(r).ok_or_else(|| PlacementError::UnknownInstance(r.to_string()))?;
        return pick(c, Why::Override);
    }
    let has = |label: &str| labels.iter().any(|l| l.trim().eq_ignore_ascii_case(label));
    if let Some(rule) = rules.iter().find(|r| has(&r.label)) {
        let c = find(&rule.instance).ok_or_else(|| PlacementError::UnknownInstance(rule.instance.clone()))?;
        return pick(c, Why::Label(rule.label.clone()));
    }
    for rule in repo {
        let Some(label) = rule.labels.iter().find(|l| has(l)) else { continue };
        if own.is_some_and(|f| rule.met_by(f)) {
            return Ok(Placed::Owner);
        }
        let matching: Vec<_> = instances.iter().filter(|c| c.facts.is_some_and(|f| rule.met_by(f))).collect();
        return match matching.iter().find(|c| c.available).or(matching.first()) {
            Some(c) => pick(c, Why::Needs(label.clone())),
            None => Err(PlacementError::NoMatch { label: label.clone(), needs: needs_text(rule) }),
        };
    }
    Ok(Placed::Owner)
}

/// `os=windows, godot`.
fn needs_text(rule: &RepoRule) -> String {
    let mut parts = vec![];
    if let Some(os) = &rule.os {
        parts.push(format!("os={os}"));
    }
    parts.extend(rule.tools.iter().cloned());
    if parts.is_empty() { "nothing".into() } else { parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::ToolFact;

    fn facts(os: &str, tools: &[&str]) -> InstanceFacts {
        InstanceFacts {
            os: os.into(),
            arch: "x86_64".into(),
            cpus: 4,
            disk_free_bytes: None,
            agents: vec![],
            tools: tools.iter().map(|t| ToolFact { name: (*t).into(), path: format!("/bin/{t}") }).collect(),
        }
    }

    fn labels(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_string()).collect()
    }

    fn rule(label: &str, instance: &str) -> LabelRule {
        LabelRule { label: label.into(), instance: instance.into() }
    }

    #[test]
    fn the_override_wins_then_label_rules_then_the_owner() {
        let win = facts("windows", &["godot"]);
        let c = [Candidate { id: "w1", name: "win-box", facts: Some(&win), available: true, paused: false }];
        let rules = [rule("windows", "w1")];
        let remote = |why| Ok(Placed::Remote { id: "w1".into(), why });
        assert_eq!(place(None, &labels(&["Windows"]), &rules, &[], &c, None), remote(Why::Label("windows".into())));
        assert_eq!(place(Some("owner"), &labels(&["windows"]), &rules, &[], &c, None), Ok(Placed::Owner));
        assert_eq!(place(Some("WIN-BOX"), &[], &[], &[], &c, None), remote(Why::Override));
        assert_eq!(place(None, &labels(&["docs"]), &rules, &[], &c, None), Ok(Placed::Owner), "unlabelled: home");
        assert_eq!(place(Some("nope"), &[], &[], &[], &c, None), Err(PlacementError::UnknownInstance("nope".into())));
        let off = [Candidate { available: false, ..c[0].clone() }];
        assert_eq!(
            place(None, &labels(&["windows"]), &rules, &[], &off, None),
            Err(PlacementError::Unavailable { id: "w1".into(), name: "win-box".into(), paused: false })
        );
        let paused = [Candidate { available: false, paused: true, ..c[0].clone() }];
        let e = place(None, &labels(&["windows"]), &rules, &[], &paused, None).unwrap_err();
        assert_eq!(e.to_string(), "win-box is paused");
    }

    #[test]
    fn repo_needs_match_the_instances_facts() {
        let (win, mac) = (facts("windows", &["godot", "git"]), facts("macos", &["git"]));
        let c = [
            Candidate { id: "m", name: "mini", facts: Some(&mac), available: true, paused: false },
            Candidate { id: "w", name: "win-box", facts: Some(&win), available: true, paused: false },
        ];
        let repo = [RepoRule { labels: labels(&["windows"]), os: Some("windows".into()), tools: labels(&["Godot"]) }];
        assert_eq!(
            place(None, &labels(&["windows"]), &[], &repo, &c, Some(&mac)),
            Ok(Placed::Remote { id: "w".into(), why: Why::Needs("windows".into()) })
        );
        assert_eq!(place(None, &labels(&["windows"]), &[], &repo, &c, Some(&win)), Ok(Placed::Owner), "owner meets it");
        let none = place(None, &labels(&["windows"]), &[], &repo, &c[..1], Some(&mac));
        assert_eq!(none, Err(PlacementError::NoMatch { label: "windows".into(), needs: "os=windows, Godot".into() }));
        assert_eq!(none.unwrap_err().to_string(), "no instance can run this (needs os=windows, Godot)");
        // The matching instance is offline: the ticket waits for it rather than going elsewhere.
        let off = [c[0].clone(), Candidate { available: false, ..c[1].clone() }];
        assert_eq!(
            place(None, &labels(&["windows"]), &[], &repo, &off, Some(&mac)),
            Err(PlacementError::Unavailable { id: "w".into(), name: "win-box".into(), paused: false })
        );
        assert!(RepoRule { os: Some("mac".into()), ..Default::default() }.met_by(&mac));
    }

    #[test]
    fn settings_are_normalised_and_refuse_repeats() {
        let s = PlacementSettings { rules: vec![rule(" Windows ", "w")] }.validate().unwrap();
        assert_eq!(s.rules, [rule("windows", "w")]);
        assert!(PlacementSettings { rules: vec![rule("a", "w"), rule("A", "x")] }.validate().is_err());
        assert!(PlacementSettings { rules: vec![rule("", "w")] }.validate().is_err());
    }
}
