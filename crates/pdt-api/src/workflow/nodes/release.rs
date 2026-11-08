//! `github.release` (PDT-194): read the latest release, or one tag, through the GitHub forge (the run's project's, else
//! GitHub's account, PDT-241). The built-in "Cut a release" polls it between waits. `repo` defaults to the run's
//! project's GitHub repository.

use super::forge_call::forge_call;
use super::repo_of::{RepoOf, check_repo, has_releases, repo_for_step, templated};
use crate::workflow::catalog::{
    BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, repo_missing, spec,
};
use pdt_core::release::parse_version;
use pdt_forge::Release;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct GithubReleaseConfig {
    /// The repository (`owner/name`); the run's project's when left out.
    #[serde(default)]
    pub repo: Option<String>,
    /// The release tagged this (`v0.2.0`); the latest release when left out.
    #[serde(default)]
    pub tag: Option<String>,
    /// With the latest: count it only when its tag is not this one (nor an older version), e.g. the latest before a
    /// release was started (`{{steps.before.tag}}`). Empty: any release counts.
    #[serde(default)]
    pub newer_than: Option<String>,
    /// Instead of the latest release, the newest published one, pre-releases included (a nightly release, PDT-210,
    /// PDT-283).
    #[serde(default)]
    pub prereleases: bool,
    /// Instead of the latest release, the newest published one (pre-releases included) whose tag matches this glob
    /// (PDT-294): `*` any text, `?` one character, `{date}` a date (`v*`, `v*-beta.*`, `snapshot-{date}`). Blank: the
    /// latest release.
    #[serde(default)]
    pub tag_pattern: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GithubReleaseOutput {
    pub tag: String,
    pub name: String,
    pub url: String,
    pub published_at: Option<String>,
}

/// Whether release `tag` counts as newer than `than` (any other tag when either isn't a version).
fn newer(tag: &str, than: Option<&str>) -> bool {
    let Some(than) = than else { return true };
    if tag == than {
        return false;
    }
    match (parse_version(tag), parse_version(than)) {
        (Some(a), Some(b)) => a.is_newer_than(&b),
        _ => true,
    }
}

pub struct GithubRelease;

impl NodeKind for GithubRelease {
    fn needs_project(&self, config: &Value) -> bool {
        repo_missing(config)
    }

    fn spec(&self) -> KindSpec {
        spec::<GithubReleaseConfig, GithubReleaseOutput>(
            "github.release",
            1,
            Category::Action,
            "Read a GitHub release",
            "Reads the latest published release (drafts and pre-releases don't count; with `prereleases`, the newest \
             one, pre-releases included; with `tagPattern`, the newest whose tag matches it, `*` any text, e.g. \
             `v*-beta.*`), or the one tagged `tag`. `found` with `tag`, `name`, `url` and \
             `publishedAt`; `none` when there is none, or with `newerThan` when the latest is still that one. \
             Between waits it polls for a release to be published.",
            &[("found", "Found"), ("none", "None")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: GithubReleaseConfig = parse(config)?;
        check_repo(&c.repo)?;
        if nonblank(&c.tag).is_some() && nonblank(&c.newer_than).is_some() {
            return Err("newerThan reads the latest release: leave tag out".into());
        }
        if nonblank(&c.tag).is_some() && c.prereleases {
            return Err("prereleases reads the newest release: leave tag out".into());
        }
        match nonblank(&c.tag_pattern) {
            Some(_) if nonblank(&c.tag).is_some() => {
                Err("tagPattern reads the newest matching release: leave tag out".into())
            }
            Some(p) if !templated(p) => pdt_core::release::TagPattern::parse(p).map(|_| ()),
            _ => Ok(()),
        }
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: GithubReleaseConfig = parse(&input)?;
            let RepoOf { repo, forge, .. } = repo_for_step(cx, &c.repo).await?;
            let tag = nonblank(&c.tag).map(String::from);
            let than = nonblank(&c.newer_than).map(String::from);
            let pattern = match nonblank(&c.tag_pattern) {
                Some(p) => Some((p.to_string(), pdt_core::release::TagPattern::parse(p)?)),
                None => None,
            };
            if cx.dry_run {
                let latest = match (&pattern, c.prereleases) {
                    (Some((p, _)), _) => format!("the newest release matching {p}"),
                    (None, true) => "the newest release or pre-release".into(),
                    (None, false) => "the latest release".into(),
                };
                let what = tag.as_ref().map_or(latest, |t| format!("release {t}"));
                return Ok(Outcome::would("found", format!("read {what} of {repo}")));
            }
            let (r, t, pre) = (repo.clone(), tag.clone(), c.prereleases);
            has_releases(&forge)?;
            let release: Option<Release> = forge_call(&forge, move |f| match (t, pattern, pre) {
                (None, Some((_, p)), _) => Ok(f.recent_releases(&r)?.into_iter().find(|x| p.matches(&x.tag).is_some())),
                (None, None, true) => f.newest_release(&r),
                (t, _, _) => f.release(&r, t.as_deref()),
            })
            .await?;
            match release {
                Some(rel) if newer(&rel.tag, than.as_deref()) => Ok(Outcome::next(
                    "found",
                    GithubReleaseOutput { tag: rel.tag, name: rel.name, url: rel.url, published_at: rel.published_at },
                )),
                other => Ok(Outcome::Next {
                    port: "none".into(),
                    output: json!({"repo": repo, "latest": other.map(|r| r.tag), "quiet": true}),
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configs_are_checked() {
        let rel = |v: Value| GithubRelease.check(&v);
        assert!(rel(json!({})).is_ok());
        assert!(rel(json!({"newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tag": "v1.0.0", "newerThan": "v0.9.0"})).is_err());
        assert!(rel(json!({"prereleases": true, "newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tag": "v1.0.0", "prereleases": true})).is_err());
        assert!(rel(json!({"tagPattern": "v*-beta.*", "newerThan": "{{steps.before.tag}}"})).is_ok());
        assert!(rel(json!({"tagPattern": "{{trigger.tag}}"})).is_ok());
        assert!(rel(json!({"tagPattern": "{date}{date}"})).is_err());
        assert!(rel(json!({"tagPattern": "v*", "tag": "v1.0.0"})).is_err());
    }

    #[test]
    fn a_release_is_newer_when_its_version_is() {
        assert!(newer("v0.2.0", None));
        assert!(newer("v0.2.0", Some("v0.1.840")));
        assert!(!newer("v0.2.0", Some("v0.2.0")));
        assert!(!newer("v0.1.9", Some("v0.2.0")));
        assert!(newer("nightly", Some("v0.2.0")));
        assert!(newer("v0.2.1-exp.1", Some("v0.2.0")));
        assert!(newer("v0.2.1-exp.2", Some("v0.2.1-exp.1")));
        assert!(!newer("v0.2.0", Some("v0.2.1-exp.1")));
    }
}
