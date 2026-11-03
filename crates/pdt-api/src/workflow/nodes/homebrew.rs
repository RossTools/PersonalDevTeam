//! `github.homebrew_cask` (PDT-373): bumps a Homebrew cask in a tap on GitHub to a release. It downloads the
//! release's macOS file for its sha256, sets the cask's `version` and `sha256` (pdt-core's `homebrew`), and opens a
//! pull request on the tap from a branch of its own (or pushes to the tap's branch). Run again (a retry, a restart), it
//! finds its branch and pull request instead of making them twice.

use super::release::{check_repo, repo_for_step};
use crate::workflow::catalog::{BoxFut, Category, KindSpec, NodeKind, Outcome, StepCx, nonblank, parse, spec};
use pdt_core::homebrew::{bump_cask, cask_path, cask_version, mac_asset};
use pdt_core::pr::PrState;
use pdt_core::workflow::events::ReleaseAssetRef;
use pdt_core::workflow::tpl::Tpl;
use pdt_forge::{FileWrite, Forge, ForgeError, NewChangeRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use ts_rs::TS;

pub const KIND: &str = "github.homebrew_cask";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct HomebrewCaskConfig {
    /// The tap: `owner/name` on GitHub (`me/homebrew-tap`).
    pub tap: String,
    /// The cask's name; its file is `Casks/<cask>.rb`.
    pub cask: String,
    /// The cask's file in the tap, when it isn't `Casks/<cask>.rb`.
    #[serde(default)]
    pub path: Option<String>,
    /// The release's tag or version (`v1.2.0` or `1.2.0`; a leading `v` is dropped).
    pub version: String,
    /// The release's files, each with `name` and `url` (`{{trigger.assets}}`).
    #[serde(default)]
    #[ts(type = "unknown")]
    pub assets: Tpl<Vec<ReleaseAssetRef>>,
    /// Which file is the macOS one, as a glob (`*.dmg`, `*-universal.zip`). Blank: the first `.dmg`, else
    /// `.app.tar.gz`, else `.app.zip`, else a `.zip` naming mac or darwin.
    #[serde(default)]
    pub asset: Option<String>,
    /// The release's page, linked from the pull request.
    #[serde(default)]
    pub release_url: Option<String>,
    /// Push the change straight to the tap's branch instead of opening a pull request.
    #[serde(default)]
    #[ts(type = "boolean | string")]
    pub push: Tpl<bool>,
    /// The tap's branch it goes to; its default branch when left out.
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HomebrewCaskOutput {
    pub version: String,
    pub sha256: String,
    /// The macOS file it was taken from.
    pub asset: String,
    /// The cask's file and the tap's branch it went to.
    pub path: String,
    pub branch: String,
    /// The commit, when it made one.
    pub commit: Option<String>,
    /// The pull request, when it opened one (or found the one an earlier try opened).
    pub number: Option<u32>,
    pub url: Option<String>,
}

/// Runs a forge call off the async threads.
async fn on_forge<T: Send + 'static>(
    forge: &Arc<dyn Forge>,
    f: impl FnOnce(&dyn Forge) -> Result<T, ForgeError> + Send + 'static,
) -> Result<T, String> {
    let forge = forge.clone();
    tokio::task::spawn_blocking(move || f(forge.as_ref()))
        .await
        .map_err(|e| format!("the GitHub task failed: {e}"))?
        .map_err(|e| format!("GitHub: {e}"))
}

/// The sha256 of the file at `url`, read as it downloads.
async fn sha256_of(url: &str) -> Result<String, String> {
    use super::http::{client, send_error};
    let mut resp =
        client().get(url).send().await.map_err(|e| format!("downloading the macOS file: {}", send_error(e)))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("downloading the macOS file: the server answered {status}"));
    }
    let mut hash = Sha256::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("downloading the macOS file: {}", send_error(e)))? {
        hash.update(&chunk);
    }
    Ok(hex::encode(hash.finalize()))
}

pub struct HomebrewCask;

impl NodeKind for HomebrewCask {
    fn spec(&self) -> KindSpec {
        spec::<HomebrewCaskConfig, HomebrewCaskOutput>(
            KIND,
            1,
            Category::Action,
            "Bump a Homebrew cask",
            "Sets a Homebrew cask in a tap on GitHub to a release: downloads the release's macOS file for its sha256, \
             sets the cask's `version` and `sha256`, and opens a pull request on the tap (or with `push`, commits to \
             its branch). `updated` with `version`, `sha256`, `commit` and the pull request's `number` and `url`; \
             `unchanged` when the cask is already at that version and checksum. The cask needs one `version \"…\"` and \
             one `sha256 \"…\"` (one file for every Mac).",
            &[("updated", "Updated"), ("unchanged", "Already up to date")],
            false,
        )
    }

    fn check(&self, config: &Value) -> Result<(), String> {
        let c: HomebrewCaskConfig = parse(config)?;
        if c.tap.trim().is_empty() {
            return Err("tap is empty: give the tap's owner/name".into());
        }
        check_repo(&Some(c.tap.clone()))?;
        if c.cask.trim().is_empty() {
            return Err("cask is empty: give the cask's name".into());
        }
        if c.version.trim().is_empty() {
            return Err("version is empty: give the release's tag ({{trigger.tag}})".into());
        }
        Ok(())
    }

    fn run<'a>(&'a self, cx: &'a StepCx<'a>, input: Value) -> BoxFut<'a, Result<Outcome, String>> {
        Box::pin(async move {
            let c: HomebrewCaskConfig = parse(&input)?;
            let version = cask_version(&c.version);
            let cask = c.cask.trim().to_string();
            let path = nonblank(&c.path).map_or_else(|| cask_path(&cask), String::from);
            let push = c.push.or(false);
            let assets = c.assets.or(vec![]);
            let names: Vec<&str> = assets.iter().map(|a| a.name.as_str()).collect();
            let picked = mac_asset(&names, nonblank(&c.asset))?;
            if cx.dry_run {
                let how = if push { "push to" } else { "open a pull request on" };
                let from = picked.map(|a| format!(" from {a}")).unwrap_or_default();
                let would = format!("set {path} to {version}{from} and {how} {}", c.tap.trim());
                return Ok(Outcome::would("updated", would));
            }
            let name = picked.ok_or_else(|| {
                format!("the release has no macOS file to take ({}): set asset to the one to take", names.join(", "))
            })?;
            let url = assets.iter().find(|a| a.name == name).map(|a| a.url.clone()).unwrap_or_default();
            let sha256 = sha256_of(&url).await?;

            let tap = repo_for_step(cx, &Some(c.tap.trim().to_string())).await?;
            let (forge, repo) = (tap.forge, tap.repo);
            let base = match nonblank(&c.branch) {
                Some(b) => b.to_string(),
                None => {
                    let r = repo.clone();
                    on_forge(&forge, move |f| f.default_branch(&r)).await?
                }
            };
            let read = |branch: String| {
                let (r, p) = (repo.clone(), path.clone());
                let forge = forge.clone();
                async move { on_forge(&forge, move |f| f.file(&r, &p, &branch)).await }
            };
            let on_base = read(base.clone()).await?.ok_or_else(|| format!("{repo} has no {path} on {base}"))?;
            let bumped = bump_cask(&on_base.text, &version, &sha256)?;
            let mut out = HomebrewCaskOutput {
                version: version.clone(),
                sha256: sha256.clone(),
                asset: name.to_string(),
                path: path.clone(),
                branch: base.clone(),
                commit: None,
                number: None,
                url: None,
            };
            if bumped == on_base.text {
                return Ok(Outcome::next("unchanged", out));
            }
            let message = format!("{cask} {version}");
            if push {
                let w = FileWrite { branch: base, path, text: bumped, message, replaces: Some(on_base.id) };
                let r = repo.clone();
                out.commit = Some(on_forge(&forge, move |f| f.write_file(&r, &w)).await?);
                return Ok(Outcome::next("updated", out));
            }

            // A branch of its own; an earlier try's branch and pull request are taken up again.
            let head = format!("{cask}-{version}");
            out.branch = head.clone();
            let (r, h) = (repo.clone(), head.clone());
            let open = on_forge(&forge, move |f| f.change_request(&r, &h)).await?.filter(|p| p.state == PrState::Open);
            if let Some(pr) = open {
                (out.number, out.url) = (Some(pr.number), Some(pr.url));
                return Ok(Outcome::next("updated", out));
            }
            let (r, h, b) = (repo.clone(), head.clone(), base.clone());
            on_forge(&forge, move |f| f.create_branch(&r, &h, &b)).await?;
            let on_head = read(head.clone()).await?;
            let replaces = on_head.as_ref().map(|f| f.id.clone());
            let now = on_head.map(|f| f.text).unwrap_or_default();
            if bump_cask(&now, &version, &sha256).ok().as_deref() != Some(now.as_str()) {
                let w = FileWrite { branch: head.clone(), path, text: bumped, message: message.clone(), replaces };
                let r = repo.clone();
                out.commit = Some(on_forge(&forge, move |f| f.write_file(&r, &w)).await?);
            }
            let link = nonblank(&c.release_url).map(|u| format!(" ([release]({u}))")).unwrap_or_default();
            let new = NewChangeRequest {
                head,
                base,
                title: message,
                body: format!(
                    "Updates the {cask} cask to {version}{link}: `sha256` of `{name}` is `{sha256}`.\n\nOpened by \
                     the {} workflow.",
                    cx.workflow.name
                ),
                draft: false,
            };
            let r = repo.clone();
            let opened = on_forge(&forge, move |f| f.open_change_request(&r, &new)).await?;
            (out.number, out.url) = (Some(opened.number), Some(opened.url));
            Ok(Outcome::next("updated", out))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_config_needs_a_tap_a_cask_and_a_version() {
        let ok = json!({"tap": "o/homebrew-tap", "cask": "thing", "version": "{{trigger.tag}}",
                        "assets": "{{trigger.assets}}", "push": "{{params.push}}"});
        HomebrewCask.check(&ok).unwrap();
        let mut bad = ok.clone();
        bad["tap"] = json!("not a repo");
        assert!(HomebrewCask.check(&bad).unwrap_err().contains("owner/name"));
        bad["tap"] = json!("");
        assert!(HomebrewCask.check(&bad).unwrap_err().contains("tap is empty"));
        let mut bad = ok.clone();
        bad["cask"] = json!(" ");
        assert!(HomebrewCask.check(&bad).unwrap_err().contains("cask"));
    }
}
