//! Releases and updates (remote-host spec §3): versions, assets, checksums, token warnings. Pure.

use crate::host::UpdateChannel;
use serde::Serialize;
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// A local build (`X.Y.Z-local`): older than every release.
    pub local: bool,
    /// A pre-release of `X.Y.Z` built from main (a nightly, or an older experimental build): older than `X.Y.Z`
    /// itself.
    pub pre: Option<Pre>,
}

/// The pre-releases PDT makes of main, in SemVer's order: `X.Y.Z-exp.N` < `X.Y.Z-nightly.D` < `X.Y.Z-nightly.D.2`
/// < `X.Y.Z` ("exp" sorts before "nightly", and fewer identifiers before more).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pre {
    /// An experimental build (`X.Y.Z-exp.N`, PDT-210), as the release workflow made them before nightlies.
    Exp(u64),
    /// A nightly build (`X.Y.Z-nightly.YYYYMMDD`, PDT-283); `build` is the day's second, third… (`.2`, `.3`), 1 for
    /// the first, which has no suffix.
    Nightly { date: u64, build: u64 },
}

impl Version {
    /// The order between releases (SemVer for the versions PDT makes): its pre-releases before `X.Y.Z`.
    pub fn key(&self) -> (u64, u64, u64, bool, Option<Pre>) {
        (self.major, self.minor, self.patch, self.pre.is_none(), self.pre)
    }

    pub fn is_newer_than(&self, current: &Version) -> bool {
        if self.local {
            return false;
        }
        current.local || self.key() > current.key()
    }

    /// A pre-release built from main: a nightly (`X.Y.Z-nightly.D`) or an older experimental build (`X.Y.Z-exp.N`).
    pub fn is_nightly(&self) -> bool {
        self.pre.is_some()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match self.pre {
            Some(Pre::Exp(n)) => write!(f, "-exp.{n}")?,
            Some(Pre::Nightly { date, build: 1 }) => write!(f, "-nightly.{date}")?,
            Some(Pre::Nightly { date, build }) => write!(f, "-nightly.{date}.{build}")?,
            None => {}
        }
        if self.local {
            f.write_str("-local")?;
        }
        Ok(())
    }
}

/// `X.Y.Z`, `vX.Y.Z`, `X.Y.Z-nightly.YYYYMMDD[.N]` (a nightly build), `X.Y.Z-exp.N` (an experimental build) or
/// `X.Y.Z-local`, with an optional `+build` suffix. Other pre-releases are not ours.
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    let s = s.split('+').next()?;
    // SemVer's numeric identifiers: digits, no leading zero.
    let number = |p: &str| {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && (p == "0" || !p.starts_with('0')))
            .then(|| p.parse::<u64>().ok())
            .flatten()
    };
    let nightly = |rest: &str| -> Option<Pre> {
        let (date, build) = match rest.split_once('.') {
            Some((d, b)) => (number(d)?, number(b).filter(|b| *b >= 2)?),
            None => (number(rest)?, 1),
        };
        (10_000_000..=99_999_999).contains(&date).then_some(Pre::Nightly { date, build })
    };
    let (core, local, pre) = match s.split_once('-') {
        Some((c, "local")) => (c, true, None),
        Some((c, p)) => match (p.strip_prefix("exp."), p.strip_prefix("nightly.")) {
            (Some(n), _) => (c, false, Some(Pre::Exp(number(n)?))),
            (_, Some(rest)) => (c, false, Some(nightly(rest)?)),
            _ => return None,
        },
        None => (s, false, None),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = Version { major: parts.next()??, minor: parts.next()??, patch: parts.next()??, local, pre };
    parts.next().is_none().then_some(v)
}

/// Whether a host on `channel` takes a published release (PDT-210, PDT-283): the stable channel only full releases,
/// the nightly one full releases and the nightly (and older experimental) pre-releases. Other pre-releases (a fork's
/// test prefix) never count.
pub fn on_channel(channel: UpdateChannel, version: &Version, prerelease: bool) -> bool {
    matches!((channel, prerelease, version.is_nightly()), (_, false, false) | (UpdateChannel::Nightly, true, true))
}

/// The newest of `releases` (version, pre-release) a host on `channel` takes, by SemVer: its index.
pub fn newest_on_channel(channel: UpdateChannel, releases: &[(Version, bool)]) -> Option<usize> {
    releases
        .iter()
        .enumerate()
        .filter(|(_, (v, pre))| !v.local && on_channel(channel, v, *pre))
        .max_by_key(|(_, (v, _))| v.key())
        .map(|(i, _)| i)
}

/// What the missing scheduled release watch (PDT-288, PDT-294) finds for a day's release: a release workflow's
/// schedule is best-effort, and GitHub sometimes never starts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledWatch {
    /// A matching release for the day (or a later one) is published.
    Published,
    /// It is not the day's due time yet: the schedule may still run.
    Early,
    /// The branch has nothing the newest matching release (or the latest release) lacks: nothing to publish.
    Unchanged,
    /// The branch changed (or that isn't asked), the due time has passed and no release for the day is published.
    Missing,
}

/// A day as `YYYYMMDD` (`20261030`) from `2026-10-30` or `20261030`.
pub fn parse_day(s: &str) -> Option<u64> {
    let digits: String = s.trim().chars().filter(|c| *c != '-').collect();
    if digits.len() != 8 {
        return None;
    }
    let d = chrono::NaiveDate::parse_from_str(&digits, "%Y%m%d").ok()?;
    d.format("%Y%m%d").to_string().parse().ok()
}

/// The UTC day of `ms` (`20261030`).
pub fn utc_day(ms: i64) -> u64 {
    let t = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default();
    t.format("%Y%m%d").to_string().parse().unwrap_or_default()
}

/// The UTC day (`20261030`) of an RFC 3339 time (a release's `publishedAt`).
pub fn rfc3339_day(s: &str) -> Option<u64> {
    let t = chrono::DateTime::parse_from_rfc3339(s.trim()).ok()?;
    Some(utc_day(t.timestamp_millis()))
}

/// When `day` (`20261030`) reaches `minutes` after midnight UTC, in ms.
pub fn day_time_ms(day: u64, minutes: u32) -> i64 {
    let Ok(d) = chrono::NaiveDate::parse_from_str(&day.to_string(), "%Y%m%d") else { return 0 };
    let m = minutes.min(24 * 60 - 1);
    d.and_hms_opt(m / 60, m % 60, 0).map_or(0, |t| t.and_utc().timestamp_millis())
}

/// A time of day, `HH:MM` (or `H:MM`, or a bare hour), as minutes after midnight.
pub fn parse_time_of_day(s: &str) -> Option<u32> {
    let s = s.trim();
    let (h, m) = s.split_once(':').unwrap_or((s, "0"));
    let all_digits = |x: &str| !x.is_empty() && x.len() <= 2 && x.chars().all(|c| c.is_ascii_digit());
    if !all_digits(h) || !all_digits(m) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// A release tag pattern (PDT-294): a glob, anchored at both ends, where `*` is any run of characters, `?` one
/// character and `{date}` (at most once) a date, `YYYYMMDD` or `YYYY-MM-DD`: `v*-nightly.{date}*`, `snapshot-{date}`,
/// `v*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagPattern {
    tokens: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Char(char),
    Any,
    One,
    Date,
}

/// The `{date}` placeholder of a [`TagPattern`].
pub const DATE_PLACEHOLDER: &str = "{date}";

impl TagPattern {
    pub fn parse(s: &str) -> Result<TagPattern, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("the tag pattern is empty".into());
        }
        let mut tokens = vec![];
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            if let Some(after) = rest.strip_prefix(DATE_PLACEHOLDER) {
                if tokens.contains(&Token::Date) {
                    return Err(format!("{s:?} has {DATE_PLACEHOLDER} twice"));
                }
                tokens.push(Token::Date);
                rest = after;
                continue;
            }
            tokens.push(match c {
                '*' => Token::Any,
                '?' => Token::One,
                c => Token::Char(c),
            });
            rest = &rest[c.len_utf8()..];
        }
        Ok(TagPattern { tokens })
    }

    /// Whether it has `{date}`.
    pub fn has_date(&self) -> bool {
        self.tokens.contains(&Token::Date)
    }

    /// None when `tag` doesn't match; else the date `{date}` matched (`20261030`), if the pattern has one.
    pub fn matches(&self, tag: &str) -> Option<Option<u64>> {
        let chars: Vec<char> = tag.chars().collect();
        match_at(&self.tokens, &chars, None)
    }
}

/// The date at the start of `s`: `YYYYMMDD` or `YYYY-MM-DD`, with how many characters it takes. Both lengths are
/// tried (longest first) by the caller's backtracking.
fn dates_at(s: &[char]) -> Vec<(u64, usize)> {
    let mut out = vec![];
    for len in [10, 8] {
        if s.len() < len {
            continue;
        }
        let text: String = s[..len].iter().collect();
        let shape = if len == 10 {
            text.char_indices().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() })
        } else {
            text.chars().all(|c| c.is_ascii_digit())
        };
        if let Some(d) = shape.then(|| parse_day(&text)).flatten() {
            out.push((d, len));
        }
    }
    out
}

fn match_at(tokens: &[Token], s: &[char], date: Option<u64>) -> Option<Option<u64>> {
    let Some((t, rest)) = tokens.split_first() else { return s.is_empty().then_some(date) };
    match t {
        Token::Char(c) => (s.first() == Some(c)).then(|| match_at(rest, &s[1..], date)).flatten(),
        Token::One => (!s.is_empty()).then(|| match_at(rest, &s[1..], date)).flatten(),
        Token::Any => (0..=s.len()).find_map(|i| match_at(rest, &s[i..], date)),
        Token::Date => dates_at(s).into_iter().find_map(|(d, len)| match_at(rest, &s[len..], Some(d))),
    }
}

/// The watch's decision for `day` at `now_ms`, `due_minutes` after midnight UTC being when its release should be out:
/// published first (so an item clears whenever one appears), then nothing before the due time, then nothing when the
/// branch didn't change.
pub fn scheduled_watch(now_ms: i64, day: u64, due_minutes: u32, published: bool, changed: bool) -> ScheduledWatch {
    if published {
        ScheduledWatch::Published
    } else if now_ms < day_time_ms(day, due_minutes) {
        ScheduledWatch::Early
    } else if !changed {
        ScheduledWatch::Unchanged
    } else {
        ScheduledWatch::Missing
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Windows,
    Mac,
    /// `arch` is Rust's name for the CPU (`std::env::consts::ARCH`: `x86_64`, `aarch64`); `appimage` when pdt-host runs
    /// the payload of an AppImage (its install folder holds [`APPIMAGE_MARKER`]), else the headless tarball.
    Linux {
        arch: &'static str,
        appimage: bool,
    },
    Other,
}

impl HostOs {
    /// This platform; on Linux, the tarball (see [`HostOs::current_linux`] for the AppImage).
    pub fn current() -> HostOs {
        Self::current_linux(false)
    }

    /// This platform, with `appimage` saying which Linux package is installed (ignored elsewhere).
    pub fn current_linux(appimage: bool) -> HostOs {
        if cfg!(windows) {
            HostOs::Windows
        } else if cfg!(target_os = "macos") {
            HostOs::Mac
        } else if cfg!(target_os = "linux") {
            HostOs::Linux { arch: std::env::consts::ARCH, appimage }
        } else {
            HostOs::Other
        }
    }
}

/// The CPUs Linux releases are built for (Rust's names, which the asset names use).
pub const LINUX_ARCHES: [&str; 2] = ["x86_64", "aarch64"];

/// The file in a Linux install folder that marks it as an AppImage's payload; it holds the AppImage's path.
pub const APPIMAGE_MARKER: &str = "appimage.path";

/// The headless Linux tarball for `arch`: `pdt-linux-x86_64.tar.gz` (a `pdt/` folder with pdtd, pdt-host and `web/`).
pub fn linux_tarball_name(arch: &str) -> String {
    format!("{}-linux-{arch}.tar.gz", crate::brand::SLUG)
}

/// The Linux AppImage for `arch`: `pdt-linux-x86_64.AppImage`.
pub fn linux_appimage_name(arch: &str) -> String {
    format!("{}-linux-{arch}.AppImage", crate::brand::SLUG)
}

/// The release asset each platform installs: the NSIS installer (`*-setup.exe`), the tarred `.app` bundle
/// (`*.app.tar.gz`), or on Linux the tarball or AppImage for this CPU, by exact name.
pub fn select_asset<'a>(names: &[&'a str], os: HostOs) -> Option<&'a str> {
    let suffix = |suffix: &str| names.iter().copied().find(|n| n.len() > suffix.len() && n.ends_with(suffix));
    match os {
        HostOs::Windows => suffix("-setup.exe"),
        HostOs::Mac => suffix(".app.tar.gz"),
        HostOs::Linux { arch, appimage } if LINUX_ARCHES.contains(&arch) => {
            let want = if appimage { linux_appimage_name(arch) } else { linux_tarball_name(arch) };
            names.iter().copied().find(|n| *n == want)
        }
        HostOs::Linux { .. } | HostOs::Other => None,
    }
}

pub const SUMS_ASSET: &str = "SHA256SUMS";

/// `sha256sum` output: `<64 hex>  <name>` or `<64 hex> *<name>` → name → lower-case hex. Other lines are ignored.
pub fn parse_sums(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (hash, rest) = l.split_once(' ')?;
            let name = rest.trim_start_matches(' ').trim_start_matches('*').trim_end();
            (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty())
                .then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub const TOKEN_WARN_MS: i64 = 14 * 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum TokenState {
    Missing,
    Valid {
        #[ts(type = "number | null")]
        expires_at: Option<i64>,
    },
    ExpiresSoon {
        #[ts(type = "number")]
        expires_at: i64,
    },
    /// Expired, or GitHub rejected it (revoked).
    Expired,
}

/// Warns `TOKEN_WARN_MS` (14 days) ahead of expiry (spec §3.2).
pub fn token_state(present: bool, expires_at: Option<i64>, rejected: bool, now: i64) -> TokenState {
    match (present, expires_at) {
        (false, _) => TokenState::Missing,
        _ if rejected => TokenState::Expired,
        (true, Some(t)) if t <= now => TokenState::Expired,
        (true, Some(t)) if t - now <= TOKEN_WARN_MS => TokenState::ExpiresSoon { expires_at: t },
        (true, t) => TokenState::Valid { expires_at: t },
    }
}

/// Where the notes for an update start (PDT-169): the installed release's tag (`v0.1.741`), or the commit of a dev
/// build (`0.1.741+g<sha>`). None for a local build without a commit, or a version that is not ours.
pub fn compare_base(current: &str) -> Option<String> {
    let current = current.trim();
    if let Some((_, build)) = current.split_once('+')
        && let Some(sha) = build.strip_prefix('g')
        && sha.len() >= 7
        && sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Some(sha.to_string());
    }
    parse_version(current).filter(|v| !v.local).map(|v| format!("v{v}"))
}

/// One commit between two versions, read from its subject (merges are squashed: one per pull request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub sha: String,
    /// The whole first line of the commit message.
    pub subject: String,
    /// The subject without its ticket key and pull request number.
    pub title: String,
    /// `PDT-167` from `PDT-167: …`.
    pub ticket: Option<String>,
    /// `177` from `… (#177)`.
    pub pr: Option<u64>,
}

/// Reads a commit: its subject is the message's first line, a leading `KEY-N:` its ticket and a trailing `(#N)` its
/// pull request.
pub fn parse_change(sha: &str, message: &str) -> Change {
    let subject = message.lines().next().unwrap_or("").trim().to_string();
    let mut rest = subject.as_str();
    let mut pr = None;
    if let Some(head) = rest.strip_suffix(')')
        && let Some((head, n)) = head.rsplit_once("(#")
        && let Ok(n) = n.parse::<u64>()
    {
        pr = Some(n);
        rest = head.trim_end();
    }
    let mut ticket = None;
    if let Some((key, after)) = rest.split_once(':')
        && let Some(k) = crate::ticket::parse_key(key)
        && let Some(project) = k.key
    {
        ticket = Some(crate::ticket::format_key(&project, k.number));
        rest = after.trim_start();
    }
    let title = if rest.is_empty() { subject.clone() } else { rest.to_string() };
    Change { sha: sha.to_string(), subject, title, ticket, pr }
}

/// Release notes cut to about `max` characters for an announcement (PDT-373): whole lines while they fit (at least
/// the first, cut short), then `…`.
pub fn notes_summary(notes: &str, max: usize) -> String {
    let notes = notes.trim();
    if notes.chars().count() <= max {
        return notes.to_string();
    }
    let mut out = String::new();
    for line in notes.lines() {
        if out.chars().count() + line.chars().count() + 1 > max {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    if out.trim().is_empty() {
        out = notes.chars().take(max.saturating_sub(1)).collect();
    }
    format!("{}\n…", out.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PDT-288, PDT-294: the missing scheduled release watch's decision.
    #[test]
    fn a_scheduled_release_is_missing_only_when_the_branch_changed_and_it_is_due() {
        let day = parse_day("2026-10-30").unwrap();
        assert_eq!(day, 20261030);
        assert_eq!(parse_day("20261030"), Some(20261030));
        assert_eq!(parse_day("2026-13-01"), None);
        assert_eq!(parse_day("{{trigger.date}}"), None);
        assert_eq!(parse_day("202610301"), None);
        let at = |h: u32| day_time_ms(day, h * 60);
        assert_eq!(at(10), 1_793_354_400_000, "2026-10-30T10:00:00Z");
        assert_eq!(day_time_ms(day, 10 * 60 + 30), 1_793_354_400_000 + 30 * 60_000);
        assert_eq!(utc_day(at(10)), day);
        assert_eq!(utc_day(at(0) - 1), 20261029);
        assert_eq!(rfc3339_day("2026-10-30T23:59:00Z"), Some(20261030));
        assert_eq!(rfc3339_day("2026-10-30T23:59:00-02:00"), Some(20261031));
        assert_eq!(rfc3339_day("soon"), None);

        assert_eq!(parse_time_of_day("10:00"), Some(600));
        assert_eq!(parse_time_of_day("7:05"), Some(425));
        assert_eq!(parse_time_of_day("9"), Some(540));
        for bad in ["24:00", "10:60", "", "ten", "10:5:0", "-1"] {
            assert_eq!(parse_time_of_day(bad), None, "{bad}");
        }

        use ScheduledWatch::*;
        // Before 10:00 UTC the schedule may still run (or run late).
        assert_eq!(scheduled_watch(at(10) - 1, day, 600, false, true), Early);
        // Due, the branch changed, none published: missing.
        assert_eq!(scheduled_watch(at(10), day, 600, false, true), Missing);
        assert_eq!(scheduled_watch(at(23), day, 600, false, true), Missing);
        // A day the branch didn't change: nothing to publish.
        assert_eq!(scheduled_watch(at(11), day, 600, false, false), Unchanged);
        // Published (even early): the item clears.
        assert_eq!(scheduled_watch(at(11), day, 600, true, true), Published);
        assert_eq!(scheduled_watch(at(9), day, 600, true, true), Published);
    }

    /// PDT-294: tag patterns, with the date `{date}` matched.
    #[test]
    fn tag_patterns_match_globs_and_find_the_date() {
        let p = |s: &str| TagPattern::parse(s).unwrap();
        let nightly = p("v*-nightly.{date}*");
        assert!(nightly.has_date());
        assert_eq!(nightly.matches("v0.2.1-nightly.20261030"), Some(Some(20261030)));
        assert_eq!(nightly.matches("v0.2.1-nightly.20261030.2"), Some(Some(20261030)));
        assert_eq!(nightly.matches("v0.2.1"), None);
        assert_eq!(nightly.matches("0.2.1-nightly.20261030"), None, "anchored: the v is needed");
        assert_eq!(nightly.matches("v0.2.1-nightly.20261399"), None, "not a date");

        let dashed = p("nightly-{date}");
        assert_eq!(dashed.matches("nightly-2026-10-30"), Some(Some(20261030)));
        assert_eq!(dashed.matches("nightly-20261030"), Some(Some(20261030)));
        assert_eq!(dashed.matches("nightly-2026-10-30-b"), None);

        let any = p("v*");
        assert!(!any.has_date());
        assert_eq!(any.matches("v1.0.0"), Some(None));
        assert_eq!(any.matches("x1.0.0"), None);
        assert_eq!(p("v?.?.?").matches("v1.2.3"), Some(None));
        assert_eq!(p("v?.?.?").matches("v1.2.30"), None);
        assert_eq!(p("{weird}").matches("{weird}"), Some(None), "only {{date}} is special");

        assert!(TagPattern::parse(" ").is_err());
        assert!(TagPattern::parse("{date}-{date}").unwrap_err().contains("twice"));
    }

    #[test]
    fn versions_and_local_builds() {
        let v = |s| parse_version(s).unwrap();
        assert_eq!(v("v0.1.42"), Version { major: 0, minor: 1, patch: 42, local: false, pre: None });
        assert_eq!(v("0.1.0-local"), Version { major: 0, minor: 1, patch: 0, local: true, pre: None });
        assert_eq!(v("0.1.3+abc"), Version { major: 0, minor: 1, patch: 3, local: false, pre: None });
        for bad in ["", "v", "1.2", "1.2.3.4", "0.1.0-rc.1", "latest", "0.x.1"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(v("0.1.10").is_newer_than(&v("0.1.9")));
        assert!(!v("0.1.9").is_newer_than(&v("0.1.9")));
        assert!(v("0.1.1").is_newer_than(&v("0.1.0-local")), "any release beats a local build");
        assert!(v("0.1.1").is_newer_than(&v("0.9.9-local")));
        assert!(!v("0.9.9-local").is_newer_than(&v("0.1.1")), "a local build never counts as newer");
        assert_eq!(v("0.1.0-local").to_string(), "0.1.0-local");
        assert_eq!(v("v0.1.7").to_string(), "0.1.7");
    }

    #[test]
    fn versions_order_by_semver_and_never_downgrade() {
        let v = |s| parse_version(s).unwrap();
        // Existing installs on the per-merge 0.1.x numbers move to the first deliberate release.
        assert!(v("v0.2.0").is_newer_than(&v("0.1.840")));
        assert!(!v("v0.1.840").is_newer_than(&v("0.2.0")), "never a downgrade");
        assert!(!v("v0.2.0").is_newer_than(&v("0.2.0")));
        assert!(v("0.2.1").is_newer_than(&v("0.2.0")));
        assert!(v("0.10.0").is_newer_than(&v("0.9.99")), "numeric, not lexical");
        assert!(v("1.0.0").is_newer_than(&v("0.99.99")));
        assert!(!v("0.99.99").is_newer_than(&v("1.0.0")));
        // Pre-releases are not ours: GitHub's latest release never is one, and a tag like this is refused.
        assert_eq!(parse_version("v0.3.0-rc.1"), None);
    }

    #[test]
    fn experimental_builds_sort_before_their_release() {
        let v = |s| parse_version(s).unwrap();
        assert_eq!(v("v0.2.1-exp.3"), Version { major: 0, minor: 2, patch: 1, local: false, pre: Some(Pre::Exp(3)) });
        assert_eq!(v("v0.2.1-exp.3").to_string(), "0.2.1-exp.3");
        assert!(v("0.2.1-exp.3").is_nightly() && !v("0.2.1").is_nightly());
        for bad in ["0.2.1-exp", "0.2.1-exp.", "0.2.1-exp.x", "0.2.1-exp.-1", "0.2.1-exp.1.2", "0.2.1-exp.1-local"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(v("0.2.1-exp.1").is_newer_than(&v("0.2.0")));
        assert!(v("0.2.1-exp.10").is_newer_than(&v("0.2.1-exp.9")), "numeric, not lexical");
        assert!(!v("0.2.1-exp.1").is_newer_than(&v("0.2.1-exp.1")));
        // The release ships: an experimental host moves to it.
        assert!(v("0.2.1").is_newer_than(&v("0.2.1-exp.3")));
        assert!(!v("0.2.1-exp.3").is_newer_than(&v("0.2.1")), "never a downgrade");
        assert!(v("0.2.2-exp.1").is_newer_than(&v("0.2.1")));
        assert!(v("0.2.1-exp.1").is_newer_than(&v("0.2.1-local")));
    }

    /// PDT-283: `X.Y.Z-nightly.YYYYMMDD`, `.2`, `.3`… for a day's later builds; above `X.Y.Z-exp.N`, below `X.Y.Z`.
    #[test]
    fn nightly_builds_sort_above_experimental_ones_and_before_their_release() {
        let v = |s| parse_version(s).unwrap();
        let n = |date, build| Some(Pre::Nightly { date, build });
        assert_eq!(v("v0.3.1-nightly.20261030").pre, n(20261030, 1));
        assert_eq!(v("0.3.1-nightly.20261030.2").pre, n(20261030, 2));
        assert_eq!(v("v0.3.1-nightly.20261030").to_string(), "0.3.1-nightly.20261030");
        assert_eq!(v("0.3.1-nightly.20261030.12").to_string(), "0.3.1-nightly.20261030.12");
        assert!(v("0.3.1-nightly.20261030").is_nightly());
        for bad in [
            "0.3.1-nightly",
            "0.3.1-nightly.",
            "0.3.1-nightly.2026103",
            "0.3.1-nightly.020261030",
            "0.3.1-nightly.20261030.",
            "0.3.1-nightly.20261030.1",
            "0.3.1-nightly.20261030.02",
            "0.3.1-nightly.20261030.2.3",
            "0.3.1-nightly.x",
            "0.3.1-nightly.20261030-local",
        ] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        let order = [
            "0.3.0",
            "0.3.1-exp.1",
            "0.3.1-exp.40",
            "0.3.1-nightly.20261029",
            "0.3.1-nightly.20261029.2",
            "0.3.1-nightly.20261029.10",
            "0.3.1-nightly.20261030",
            "0.3.1",
            "0.3.2-exp.1",
            "0.3.2-nightly.20261101",
        ];
        for w in order.windows(2) {
            assert!(v(w[1]).is_newer_than(&v(w[0])), "{} after {}", w[1], w[0]);
            assert!(!v(w[0]).is_newer_than(&v(w[1])), "never a downgrade from {} to {}", w[1], w[0]);
        }
        // A host on an experimental build gets the next nightly of the same version.
        assert!(v("0.3.1-nightly.20261030").is_newer_than(&v("0.3.1-exp.7")));
        assert!(v("0.3.1-nightly.20261030").is_newer_than(&v("0.3.1-local")));
    }

    #[test]
    fn each_channel_takes_its_releases() {
        use UpdateChannel::{Nightly, Stable};
        let v = |s| parse_version(s).unwrap();
        let list = [(v("0.2.0"), false), (v("0.2.1-exp.2"), true), (v("0.2.1-exp.1"), true), (v("0.1.9"), false)];
        assert_eq!(newest_on_channel(Stable, &list), Some(0));
        assert_eq!(newest_on_channel(Nightly, &list), Some(1), "the old experimental builds count");
        // The first nightly after the switch is newer than the experimental builds.
        let switched = [(v("0.2.1-nightly.20261030"), true), (v("0.2.1-exp.2"), true), (v("0.2.0"), false)];
        assert_eq!(newest_on_channel(Nightly, &switched), Some(0));
        assert_eq!(newest_on_channel(Stable, &switched), Some(2));
        // The stable release sorts above its pre-releases: the nightly channel moves to it.
        let shipped = [(v("0.2.1-nightly.20261031.2"), true), (v("0.2.1"), false), (v("0.2.1-exp.2"), true)];
        assert_eq!(newest_on_channel(Nightly, &shipped), Some(1));
        // A pre-release that is not a nightly build (a fork's test prefix, an rc) is never taken, nor a nightly tag
        // published as a full release on the stable channel.
        assert!(!on_channel(Nightly, &v("0.3.0"), true));
        assert!(!on_channel(Stable, &v("0.2.1-exp.1"), true));
        assert!(!on_channel(Stable, &v("0.2.1-nightly.20261030"), true));
        assert!(!on_channel(Stable, &v("0.2.1-nightly.20261030"), false));
        assert_eq!(newest_on_channel(Stable, &[(v("0.2.1-nightly.20261030"), true)]), None);
        assert_eq!(newest_on_channel(Nightly, &[]), None);
    }

    #[test]
    fn asset_selection_matches_ci_names() {
        let names = ["Personal.Dev.Team_0.1.7_x64-setup.exe", "Personal.Dev.Team.app.tar.gz", "SHA256SUMS"];
        assert_eq!(select_asset(&names, HostOs::Windows), Some("Personal.Dev.Team_0.1.7_x64-setup.exe"));
        assert_eq!(select_asset(&names, HostOs::Mac), Some("Personal.Dev.Team.app.tar.gz"));
        assert_eq!(select_asset(&names, HostOs::Other), None);
        assert_eq!(select_asset(&["-setup.exe"], HostOs::Windows), None);
        assert_eq!(select_asset(&names, HostOs::Linux { arch: "x86_64", appimage: false }), None);
    }

    #[test]
    fn linux_picks_its_own_package_and_cpu() {
        let names = [
            "Personal.Dev.Team_0.2.0_x64-setup.exe",
            "Personal.Dev.Team.app.tar.gz",
            "pdt-linux-x86_64.tar.gz",
            "pdt-linux-aarch64.tar.gz",
            "pdt-linux-x86_64.AppImage",
            "pdt-linux-aarch64.AppImage",
            "SHA256SUMS",
        ];
        let linux = |arch, appimage| select_asset(&names, HostOs::Linux { arch, appimage });
        assert_eq!(linux("x86_64", false), Some("pdt-linux-x86_64.tar.gz"));
        assert_eq!(linux("aarch64", false), Some("pdt-linux-aarch64.tar.gz"));
        assert_eq!(linux("x86_64", true), Some("pdt-linux-x86_64.AppImage"));
        assert_eq!(linux("aarch64", true), Some("pdt-linux-aarch64.AppImage"));
        assert_eq!(linux("riscv64", false), None, "no release for other CPUs");
        // The macOS tarball is never taken for the Linux one.
        assert_eq!(select_asset(&names[..2], HostOs::Linux { arch: "x86_64", appimage: false }), None);
        if cfg!(target_os = "linux") {
            assert_eq!(HostOs::current_linux(true), HostOs::Linux { arch: std::env::consts::ARCH, appimage: true });
        }
    }

    #[test]
    fn sums_file() {
        let text = "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789  a-setup.exe\n\
                    0000000000000000000000000000000000000000000000000000000000000000 *b.app.tar.gz\n\
                    garbage line\n";
        let s = parse_sums(text);
        assert_eq!(s["a-setup.exe"], "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789");
        assert_eq!(s["b.app.tar.gz"], "0".repeat(64));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn token_warnings() {
        let day = 86_400_000;
        assert_eq!(token_state(false, None, false, 0), TokenState::Missing);
        assert_eq!(token_state(true, None, false, 0), TokenState::Valid { expires_at: None });
        assert_eq!(token_state(true, Some(30 * day), false, 0), TokenState::Valid { expires_at: Some(30 * day) });
        assert_eq!(token_state(true, Some(14 * day), false, 0), TokenState::ExpiresSoon { expires_at: 14 * day });
        assert_eq!(token_state(true, Some(10), false, 20), TokenState::Expired);
        assert_eq!(
            token_state(true, Some(30 * day), true, 0),
            TokenState::Expired,
            "rejected means expired or revoked"
        );
        assert_eq!(
            serde_json::to_string(&TokenState::ExpiresSoon { expires_at: 5 }).unwrap(),
            r#"{"state":"expires_soon","expiresAt":5}"#
        );
    }

    #[test]
    fn compare_starts_at_the_installed_tag_or_a_dev_builds_commit() {
        assert_eq!(compare_base("0.1.741").as_deref(), Some("v0.1.741"));
        assert_eq!(compare_base("0.1.741+gabc1234def").as_deref(), Some("abc1234def"));
        assert_eq!(compare_base("0.1.0-local+g0123456").as_deref(), Some("0123456"));
        // A build suffix that is not a commit leaves the version's tag.
        assert_eq!(compare_base("0.1.3+abc").as_deref(), Some("v0.1.3"));
        assert_eq!(compare_base("0.1.3+gxyz").as_deref(), Some("v0.1.3"));
        assert_eq!(compare_base("0.1.0-local"), None);
        assert_eq!(compare_base("garbage"), None);
    }

    #[test]
    fn a_squashed_merge_names_its_ticket_and_pull_request() {
        let c = parse_change("abc", "PDT-167: PM conversations compact (#177)\n\nThe body.");
        assert_eq!(
            c,
            Change {
                sha: "abc".into(),
                subject: "PDT-167: PM conversations compact (#177)".into(),
                title: "PM conversations compact".into(),
                ticket: Some("PDT-167".into()),
                pr: Some(177),
            }
        );
        let c = parse_change("d", "Fix the build (#12)");
        assert_eq!((c.title.as_str(), c.ticket, c.pr), ("Fix the build", None, Some(12)));
        let c = parse_change("d", "pdt-3: lower-case key");
        assert_eq!((c.title.as_str(), c.ticket.as_deref(), c.pr), ("lower-case key", Some("PDT-3"), None));
        let c = parse_change("d", "Note: not a key (#x)");
        assert_eq!((c.title.as_str(), c.ticket, c.pr), ("Note: not a key (#x)", None, None));
        assert_eq!(parse_change("d", "").title, "");
    }

    #[test]
    fn notes_are_cut_at_a_line_for_an_announcement() {
        assert_eq!(notes_summary("  short\n", 100), "short");
        let notes = "### Added\n- one\n- two\n- three";
        assert_eq!(notes_summary(notes, 20), "### Added\n- one\n…");
        assert_eq!(notes_summary("abcdefghij", 5), "abcd\n…", "a first line too long is cut");
    }
}
