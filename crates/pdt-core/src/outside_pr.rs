//! Pull requests PDT didn't open (PDT-268): who opened one ([`AuthorKind`]), and what a Dependabot update bumps
//! ([`dependabot_update`]). Pure: pdt-api's sensor reads them through the forge.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Who opened a pull request, for a workflow to branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AuthorKind {
    /// Not the repository's owner, a member of its organization or a collaborator on it.
    Outside,
    /// A `[bot]` account (`dependabot[bot]`, `renovate[bot]`).
    Bot,
    /// The owner, a member or a collaborator.
    Collaborator,
}

impl AuthorKind {
    /// A bot is a bot whatever its standing on the repository; else a collaborator or an outsider.
    pub fn of(is_bot: bool, collaborator: bool) -> Self {
        match (is_bot, collaborator) {
            (true, _) => AuthorKind::Bot,
            (false, true) => AuthorKind::Collaborator,
            (false, false) => AuthorKind::Outside,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AuthorKind::Outside => "outside",
            AuthorKind::Bot => "bot",
            AuthorKind::Collaborator => "collaborator",
        }
    }
}

/// What a dependency update bumps, by semantic versioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum UpdateType {
    Major,
    Minor,
    Patch,
}

/// Dependabot's login.
pub const DEPENDABOT: &str = "dependabot[bot]";

/// What a Dependabot pull request bumps, from its title (`Bump serde from 1.0.1 to 1.0.2`, `chore(deps): bump x from
/// 1.2 to 2.0 in /web`). None for another author, a grouped update (`Bump the npm group with 3 updates`) or versions
/// it can't compare. A leading `v` is dropped; a missing part counts as 0; a pre-1.0 minor bump counts as major and a
/// pre-1.0 patch bump as minor, as Cargo and npm treat them.
pub fn dependabot_update(author: &str, title: &str) -> Option<UpdateType> {
    if !author.eq_ignore_ascii_case(DEPENDABOT) {
        return None;
    }
    let lower = title.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let from = words.iter().position(|w| *w == "from")?;
    let (old, to, new) = (words.get(from + 1)?, words.get(from + 2)?, words.get(from + 3)?);
    if *to != "to" {
        return None;
    }
    let (old, new) = (version(old)?, version(new)?);
    if old == new {
        return None;
    }
    let changed = (0..3).find(|i| old[*i] != new[*i])?;
    Some(match (changed, old[0], old[1]) {
        (0, _, _) => UpdateType::Major,
        (1, 0, _) => UpdateType::Major,
        (1, _, _) => UpdateType::Minor,
        (_, 0, 0) => UpdateType::Major,
        (_, 0, _) => UpdateType::Minor,
        _ => UpdateType::Patch,
    })
}

/// `1.2.3` (`v1.2`, `1.2.3-beta`, `1.2.3.`) as three numbers.
fn version(v: &str) -> Option<[u64; 3]> {
    let v = v.trim_end_matches(['.', ',', ';']).trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut out = [0u64; 3];
    let mut parts = core.split('.');
    for slot in out.iter_mut() {
        match parts.next() {
            Some(p) => *slot = p.parse().ok()?,
            None => break,
        }
    }
    (!core.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_is_a_bot_before_it_is_a_collaborator() {
        assert_eq!(AuthorKind::of(true, true), AuthorKind::Bot);
        assert_eq!(AuthorKind::of(false, true), AuthorKind::Collaborator);
        assert_eq!(AuthorKind::of(false, false), AuthorKind::Outside);
        assert_eq!(serde_json::to_value(AuthorKind::Outside).unwrap(), "outside");
    }

    #[test]
    fn dependabot_titles_give_the_update_type() {
        let d = |t: &str| dependabot_update(DEPENDABOT, t);
        assert_eq!(d("Bump serde from 1.0.1 to 1.0.2"), Some(UpdateType::Patch));
        assert_eq!(d("Bump serde from 1.0.1 to 1.1.0"), Some(UpdateType::Minor));
        assert_eq!(d("Bump serde from 1.9.1 to 2.0.0"), Some(UpdateType::Major));
        assert_eq!(d("chore(deps): bump vite from 5.4 to 6.0 in /web"), Some(UpdateType::Major));
        assert_eq!(d("Bump actions/checkout from v3 to v4"), Some(UpdateType::Major));
        assert_eq!(d("Bump x from 1.2.3-beta.1 to 1.2.3"), None, "same release");
        assert_eq!(d("Bump x from 1.2.3 to 1.2.4."), Some(UpdateType::Patch));
    }

    #[test]
    fn pre_1_0_bumps_count_one_step_up() {
        let d = |t: &str| dependabot_update(DEPENDABOT, t);
        assert_eq!(d("Bump tokio-x from 0.3.1 to 0.4.0"), Some(UpdateType::Major));
        assert_eq!(d("Bump tokio-x from 0.3.1 to 0.3.2"), Some(UpdateType::Minor));
        assert_eq!(d("Bump tokio-x from 0.0.1 to 0.0.2"), Some(UpdateType::Major));
    }

    #[test]
    fn other_titles_and_authors_have_none() {
        assert_eq!(dependabot_update("al", "Bump serde from 1.0.1 to 1.0.2"), None);
        let d = |t: &str| dependabot_update(DEPENDABOT, t);
        assert_eq!(d("Bump the npm group across 1 directory with 3 updates"), None);
        assert_eq!(d("Bump serde from abc to 1.0.2"), None);
        assert_eq!(d("Bump serde from 1.0.1"), None);
        assert_eq!(d("Bump serde from 1.0.1 into 1.0.2"), None);
    }
}
