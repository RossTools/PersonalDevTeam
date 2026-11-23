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

impl UpdateType {
    /// The larger of two bumps: major over minor over patch.
    pub fn larger(self, other: UpdateType) -> UpdateType {
        let rank = |t: UpdateType| match t {
            UpdateType::Patch => 0,
            UpdateType::Minor => 1,
            UpdateType::Major => 2,
        };
        if rank(other) > rank(self) { other } else { self }
    }
}

/// Dependabot's login.
pub const DEPENDABOT: &str = "dependabot[bot]";

/// What a Dependabot pull request bumps, from its title (`Bump serde from 1.0.1 to 1.0.2`, `chore(deps): bump x from
/// 1.2 to 2.0 in /web`), or for a grouped update (`Bump the cargo group with 3 updates`) the largest of its body's
/// `Updates \`x\` from 1.2 to 1.3` lines ([`group_update`]). None for another author or versions it can't compare. A
/// leading `v` is dropped; a missing part counts as 0; a pre-1.0 minor bump counts as major and a pre-1.0 patch bump as
/// minor, as Cargo and npm treat them.
pub fn dependabot_update(author: &str, title: &str, body: &str) -> Option<UpdateType> {
    if !author.eq_ignore_ascii_case(DEPENDABOT) {
        return None;
    }
    let lower = title.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    if let Some(n) = group_size(&words) {
        return group_update(body, n);
    }
    let from = words.iter().position(|w| *w == "from")?;
    bump(&words[from + 1..])
}

/// How many updates a grouped update's title says it has (`bump the npm group across 2 directories with 3 updates`).
fn group_size(words: &[&str]) -> Option<usize> {
    let the = words.iter().position(|w| *w == "bump")?;
    if words.get(the + 1) != Some(&"the") || !words[the..].contains(&"group") {
        return None;
    }
    let with = words.iter().rposition(|w| *w == "with")?;
    let n = words.get(with + 1)?.parse().ok()?;
    words.get(with + 2).is_some_and(|w| w.trim_end_matches(['.', ',']).starts_with("update")).then_some(n)
}

/// The largest bump of a grouped update's body: each line `Updates \`name\` from 1.2.3 to 1.3.0` is one package's.
/// None unless it names at least `n` packages (the title's count) and every line gives versions it can compare, so a
/// body cut short, or one whose updates only widen a requirement, is reviewed. Dependabot quotes release notes as
/// HTML, so a line from them can only add a package, never make a bump smaller.
pub fn group_update(body: &str, n: usize) -> Option<UpdateType> {
    let mut names = std::collections::BTreeSet::new();
    let mut most: Option<UpdateType> = None;
    for line in body.lines().map(str::trim_end) {
        let Some(rest) = line.strip_prefix("Updates `") else { continue };
        let (name, rest) = rest.split_once('`')?;
        let words: Vec<String> = rest.to_ascii_lowercase().split_whitespace().map(String::from).collect();
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        if words.first() != Some(&"from") {
            return None;
        }
        let t = bump(&words[1..])?;
        names.insert(name.to_string());
        most = Some(most.map_or(t, |m| m.larger(t)));
    }
    (n > 0 && names.len() >= n).then_some(most).flatten()
}

/// `1.2 to 1.3` (the words after `from`) as what it bumps.
fn bump(words: &[&str]) -> Option<UpdateType> {
    let (old, to, new) = (words.first()?, words.get(1)?, words.get(2)?);
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
        let d = |t: &str| dependabot_update(DEPENDABOT, t, "");
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
        let d = |t: &str| dependabot_update(DEPENDABOT, t, "");
        assert_eq!(d("Bump tokio-x from 0.3.1 to 0.4.0"), Some(UpdateType::Major));
        assert_eq!(d("Bump tokio-x from 0.3.1 to 0.3.2"), Some(UpdateType::Minor));
        assert_eq!(d("Bump tokio-x from 0.0.1 to 0.0.2"), Some(UpdateType::Major));
    }

    #[test]
    fn other_titles_and_authors_have_none() {
        assert_eq!(dependabot_update("al", "Bump serde from 1.0.1 to 1.0.2", ""), None);
        let d = |t: &str| dependabot_update(DEPENDABOT, t, "");
        assert_eq!(d("Bump the npm group across 1 directory with 3 updates"), None, "no body");
        assert_eq!(d("Bump serde from abc to 1.0.2"), None);
        assert_eq!(d("Bump serde from 1.0.1"), None);
        assert_eq!(d("Bump serde from 1.0.1 into 1.0.2"), None);
    }

    /// The real bodies of Dependabot's grouped updates (#632, #633, #636), their release notes cut.
    const ACTIONS: &str = include_str!("../tests/fixtures/dependabot/actions-group.md");
    const PNPM: &str = include_str!("../tests/fixtures/dependabot/pnpm-group.md");
    const CARGO: &str = include_str!("../tests/fixtures/dependabot/cargo-group.md");

    #[test]
    fn a_grouped_update_is_its_largest_bump() {
        let d = |t: &str, b: &str| dependabot_update(DEPENDABOT, t, b);
        let actions = "deps: Bump the actions-minor-patch group across 2 directories with 1 update";
        assert_eq!(d(actions, ACTIONS), Some(UpdateType::Patch));
        assert_eq!(d("deps: Bump the pnpm-minor-patch group with 8 updates", PNPM), Some(UpdateType::Minor));
        // sqlx 0.9.0 to 0.9.1 is a pre-1.0 patch: minor.
        assert_eq!(d("deps: Bump the cargo-minor-patch group with 7 updates", CARGO), Some(UpdateType::Minor));
        let major = format!("{CARGO}\nUpdates `jiff` from 0.2.46 to 1.0.1\n");
        assert_eq!(d("deps: Bump the cargo group with 8 updates", &major), Some(UpdateType::Major));
    }

    #[test]
    fn a_grouped_update_it_cant_read_whole_is_none() {
        let d = |t: &str, b: &str| dependabot_update(DEPENDABOT, t, b);
        // Fewer packages than the title counts: the body was cut short.
        assert_eq!(d("Bump the cargo-minor-patch group with 9 updates", CARGO), None);
        let cut = CARGO.lines().take(20).collect::<Vec<_>>().join("\n");
        assert_eq!(d("Bump the cargo-minor-patch group with 7 updates", &cut), None);
        // A requirement widened, with no versions to compare.
        let widened = "Updates the requirements on [x](u) to permit the latest version.\nUpdates `x` to 1.2.3\n";
        assert_eq!(d("Bump the npm group with 1 update", widened), None);
        assert_eq!(d("Bump the npm group with 1 update", ""), None);
        assert_eq!(dependabot_update("al", "Bump the cargo-minor-patch group with 7 updates", CARGO), None);
    }
}
