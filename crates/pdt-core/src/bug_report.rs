//! Bug reports (PDT-153): where "Report a bug" sends a report, what its diagnostics hold, and how they are redacted
//! before anyone sees them. Pure: pdt-api gathers the facts, files the issue or ticket.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The host setting: where reports go. `Auto` is GitHub, except on an instance with a project whose origin is the
/// release repository (the instance PDT is developed on), where it is a local ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BugReportSetting {
    #[default]
    Auto,
    Github,
    Local,
    Off,
}

/// Where a report actually goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BugReportRoute {
    /// An issue on the release repository.
    Github,
    /// A Draft ticket labelled `bug` in the project whose origin is the release repository; never a GitHub request.
    Local,
    Off,
}

impl BugReportSetting {
    /// The route: `Auto` is `Local` when a project here has the release repository as its origin, else `Github`.
    pub fn route(self, has_release_project: bool) -> BugReportRoute {
        match self {
            BugReportSetting::Auto if has_release_project => BugReportRoute::Local,
            BugReportSetting::Auto | BugReportSetting::Github => BugReportRoute::Github,
            BugReportSetting::Local => BugReportRoute::Local,
            BugReportSetting::Off => BugReportRoute::Off,
        }
    }
}

/// The labels an issue gets; a local ticket gets [`TICKET_LABEL`].
pub const ISSUE_LABELS: [&str; 2] = ["bug", "from-pdt"];
pub const TICKET_LABEL: &str = "bug";
/// How many lines of the pdtd log the diagnostics end with.
pub const LOG_LINES: usize = 50;
/// How long a report's nonce is remembered: the same nonce again within it gets the first report back.
pub const NONCE_TTL_MS: i64 = 60_000;
const MAX_TITLE: usize = crate::limits::TITLE;
const MAX_DESCRIPTION: usize = 20_000;

/// The sections of a report's body, in order: the same labels as the bug form on GitHub
/// (`.github/ISSUE_TEMPLATE/bug.yml`), so bugs filed in the app and on GitHub look alike.
pub const WHAT_HAPPENED: &str = "What happened";
pub const EXPECTED: &str = "What you expected";
pub const STEPS: &str = "How to reproduce";
pub const VERSION: &str = "PDT version";
pub const OS: &str = "OS";
pub const SECTIONS: [&str; 5] = [WHAT_HAPPENED, EXPECTED, STEPS, VERSION, OS];
/// What GitHub's issue forms write for a field left empty.
const NO_RESPONSE: &str = "_No response_";

/// What the reporter wrote, by section (Markdown each).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub what_happened: String,
    pub expected: String,
    pub steps: String,
}

/// The title and report trimmed, or why they can't be sent.
pub fn validate(title: &str, report: &Report) -> Result<(String, Report), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("give the report a title".into());
    }
    if title.contains(['\n', '\r']) || title.chars().count() > MAX_TITLE {
        return Err(format!("the title must be one line of at most {MAX_TITLE} characters"));
    }
    let report = Report {
        what_happened: report.what_happened.trim().to_string(),
        expected: report.expected.trim().to_string(),
        steps: report.steps.trim().to_string(),
    };
    let len = [&report.what_happened, &report.expected, &report.steps].iter().map(|s| s.chars().count()).sum::<usize>();
    if len > MAX_DESCRIPTION {
        return Err(format!("the description must be at most {MAX_DESCRIPTION} characters"));
    }
    Ok((title.to_string(), report))
}

/// What the diagnostics say about this instance. No ticket or project content.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Diagnostics {
    pub version: String,
    /// `stable` or `dev`.
    pub profile: String,
    pub dev: bool,
    pub os: String,
    pub arch: String,
    pub supervised: bool,
    /// The update state (`idle`, `available`, `disabled`…) and its last error.
    pub update: String,
    /// `missing`, `valid`, `expiring`, `expired`.
    pub token: String,
    /// The last lines of the pdtd log, oldest first.
    pub log_tail: Vec<String>,
}

impl Diagnostics {
    /// The diagnostics as text. Only `log_tail` (and the update error) can hold anything private: redact it before it
    /// goes in.
    pub fn render(&self) -> String {
        let mut out = format!(
            "PDT version: {}\nProfile: {}{}\nOS: {} ({})\nSupervised by pdt-host: {}\nUpdates: {}\nUpdate token: {}\n",
            self.version,
            self.profile,
            if self.dev { " (development instance)" } else { "" },
            self.os,
            self.arch,
            if self.supervised { "yes" } else { "no" },
            self.update,
            self.token,
        );
        if self.log_tail.is_empty() {
            out.push_str("\npdtd log: (none found)\n");
        } else {
            out.push_str(&format!("\nLast {} lines of the pdtd log:\n", self.log_tail.len()));
            for l in &self.log_tail {
                out.push_str(l);
                out.push('\n');
            }
        }
        out
    }
}

/// The last `n` lines of `text`.
pub fn tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

/// The issue's (or ticket's) body: the report's sections, the version, the OS (`None`: not shared, as with the
/// diagnostics), the diagnostics in a collapsed block, and where it came from.
pub fn body(report: &Report, diagnostics: Option<&str>, version: &str, os: Option<&str>) -> String {
    let mut out = String::new();
    for (label, text) in [
        (WHAT_HAPPENED, report.what_happened.as_str()),
        (EXPECTED, report.expected.as_str()),
        (STEPS, report.steps.as_str()),
        (VERSION, version),
        (OS, os.unwrap_or("_Not shared_")),
    ] {
        let text = if text.trim().is_empty() { NO_RESPONSE } else { text };
        out.push_str(&format!("### {label}\n\n{text}\n\n"));
    }
    if let Some(d) = diagnostics {
        let fence = "`".repeat(longest_backtick_run(d).max(2) + 1);
        out.push_str(&format!(
            "<details><summary>Diagnostics</summary>\n\n{fence}text\n{}\n{fence}\n\n</details>\n\n",
            d.trim_end()
        ));
    }
    out.push_str(&format!("_Reported with Report a bug in {} {version}._\n", crate::brand::PRODUCT_NAME));
    out
}

fn longest_backtick_run(s: &str) -> usize {
    s.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

/// What [`redact`] takes out besides what looks like a secret anywhere.
#[derive(Debug, Clone, Default)]
pub struct Redactions {
    /// Secrets known by value (the update token): replaced wherever they appear.
    pub secrets: Vec<String>,
    /// The home folder: written as `~`.
    pub home: Option<String>,
    /// This machine's names (hostname, tailnet name): written as `[host]`.
    pub hostnames: Vec<String>,
    /// The user's own names: projects, repositories (`owner/name`), branches, worktree and repository folders. Written
    /// as `[name]` where they stand as a whole word (case as written), so a project called `web` leaves `website` be.
    pub names: Vec<String>,
}

pub const REDACTED: &str = "[redacted]";
const HOST: &str = "[host]";
pub const NAME: &str = "[name]";
/// Names too common to take out: default branches and PDT's own.
const KEEP_NAMES: [&str; 8] =
    ["main", "master", "develop", "trunk", "HEAD", "origin", crate::brand::SLUG, crate::brand::DAEMON_BIN];
/// Prefixes of well-known token formats (GitHub, OpenAI/Anthropic, Slack, AWS).
const TOKEN_PREFIXES: [&str; 10] =
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_", "sk-", "xoxb-", "xoxp-", "AKIA"];
/// Key names whose value is a secret (`token=…`, `"apiKey": "…"`, `Authorization: …`).
const SECRET_KEYS: [&str; 8] =
    ["token", "secret", "password", "passwd", "apikey", "api_key", "api-key", "authorization"];

/// `text` without secrets, home paths or this machine's names. Lossy on purpose: a long random-looking word goes too.
pub fn redact(text: &str, r: &Redactions) -> String {
    let mut s = text.to_string();
    for secret in r.secrets.iter().map(|x| x.trim()).filter(|x| x.len() >= 6) {
        s = s.replace(secret, REDACTED);
    }
    if let Some(home) = r.home.as_deref().map(|h| h.trim_end_matches(['/', '\\'])).filter(|h| h.len() > 1) {
        s = s.replace(home, "~");
        let flipped = if home.contains('\\') { home.replace('\\', "/") } else { home.replace('/', "\\") };
        s = s.replace(&flipped, "~");
    }
    for host in r.hostnames.iter().map(|h| h.trim()).filter(|h| h.len() >= 2) {
        s = replace_ignore_case(&s, host, HOST);
    }
    let mut names: Vec<&str> = r
        .names
        .iter()
        .map(|n| n.trim())
        .filter(|n| n.chars().count() >= 3 && !KEEP_NAMES.iter().any(|k| k.eq_ignore_ascii_case(n)))
        .collect();
    // Longest first, so `owner/repo` goes before `repo` and a branch before the ticket key in it.
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names {
        s = replace_word(&s, name, NAME);
    }
    s = user_folders(&s);
    s.lines().map(redact_line).collect::<Vec<_>>().join("\n") + if text.ends_with('\n') { "\n" } else { "" }
}

fn replace_ignore_case(hay: &str, needle: &str, with: &str) -> String {
    let (h, n) = (hay.to_ascii_lowercase(), needle.to_ascii_lowercase());
    let mut out = String::with_capacity(hay.len());
    let mut at = 0;
    while let Some(i) = h[at..].find(&n) {
        out.push_str(&hay[at..at + i]);
        out.push_str(with);
        at += i + n.len();
    }
    out.push_str(&hay[at..]);
    out
}

/// `needle` replaced where it isn't part of a longer word (letters, digits, `_` and `-` either side; `.` and `/`
/// end a word, so `repo` goes from `repo.git` and `owner/repo`, and `-<digit>` after it does too, so a project key
/// goes from its ticket keys).
fn replace_word(hay: &str, needle: &str, with: &str) -> String {
    let joined = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let before = |s: &str| s.chars().next_back().is_some_and(joined);
    let after = |s: &str| {
        let mut cs = s.chars();
        match cs.next() {
            Some('-') => !cs.next().is_some_and(|c| c.is_ascii_digit()),
            c => c.is_some_and(joined),
        }
    };
    let mut out = String::with_capacity(hay.len());
    let mut at = 0;
    while let Some(i) = hay[at..].find(needle) {
        let (start, end) = (at + i, at + i + needle.len());
        out.push_str(&hay[at..start]);
        if before(&hay[..start]) || after(&hay[end..]) {
            out.push_str(needle);
        } else {
            out.push_str(with);
        }
        at = end;
    }
    out.push_str(&hay[at..]);
    out
}

/// Another user's (or an unknown) home folder: `/Users/<name>`, `/home/<name>`, `C:\Users\<name>` → `<name>` hidden.
fn user_folders(s: &str) -> String {
    let mut out = s.to_string();
    for prefix in ["/Users/", "/home/", "\\Users\\", "\\users\\"] {
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(i) = rest.find(prefix) {
            result.push_str(&rest[..i + prefix.len()]);
            rest = &rest[i + prefix.len()..];
            let end = rest.find(['/', '\\', ' ', '"', '\'', ':', '\n', ')', ',']).unwrap_or(rest.len());
            if end > 0 && !rest.starts_with("[user]") && &rest[..end] != "Shared" {
                result.push_str("[user]");
                rest = &rest[end..];
            }
        }
        result.push_str(rest);
        out = result;
    }
    out
}

enum Seg {
    Word(String),
    Other(char),
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

fn segments(line: &str) -> Vec<Seg> {
    let mut out = vec![];
    let mut word = String::new();
    for c in line.chars() {
        if is_word(c) {
            word.push(c);
        } else {
            if !word.is_empty() {
                out.push(Seg::Word(std::mem::take(&mut word)));
            }
            out.push(Seg::Other(c));
        }
    }
    if !word.is_empty() {
        out.push(Seg::Word(word));
    }
    out
}

fn looks_secret(w: &str) -> bool {
    let token = TOKEN_PREFIXES.iter().any(|p| w.starts_with(p)) && w.len() >= 16;
    let random = w.len() >= 40
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().any(|c| c.is_ascii_alphabetic())
        && !w.contains('.');
    token || random
}

fn redact_line(line: &str) -> String {
    let segs = segments(line);
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < segs.len() {
        match &segs[i] {
            Seg::Other(c) => {
                out.push(*c);
                i += 1;
            }
            Seg::Word(w) => {
                let lower = w.to_ascii_lowercase();
                if lower.ends_with(".ts.net") {
                    out.push_str(HOST);
                } else if looks_secret(w) {
                    out.push_str(REDACTED);
                } else {
                    out.push_str(w);
                }
                i += 1;
                if is_auth_scheme(&lower) && followed_by_word(&segs, i) {
                    out.push(' ');
                    i = skip_value(&segs, i + 1);
                    out.push_str(REDACTED);
                } else if SECRET_KEYS.iter().any(|k| lower.contains(k))
                    && let Some(start) = value_start(&segs, i)
                {
                    for s in &segs[i..start] {
                        if let Seg::Other(c) = s {
                            out.push(*c);
                        }
                    }
                    i = skip_value(&segs, start);
                    // `Authorization: Bearer <token>`: the scheme and its token both go.
                    if let Some(Seg::Word(v)) = segs.get(start)
                        && i == start + 1
                        && is_auth_scheme(&v.to_ascii_lowercase())
                        && followed_by_word(&segs, i)
                    {
                        i = skip_value(&segs, i + 1);
                    }
                    out.push_str(REDACTED);
                }
            }
        }
    }
    out
}

fn is_auth_scheme(lower: &str) -> bool {
    lower == "bearer" || lower == "basic"
}

/// A single space then a word at `i`.
fn followed_by_word(segs: &[Seg], i: usize) -> bool {
    matches!(segs.get(i), Some(Seg::Other(' '))) && matches!(segs.get(i + 1), Some(Seg::Word(_)))
}

/// After a secret key at `i`: where its value starts, when what follows is a `=` or `:` (with quotes and spaces).
fn value_start(segs: &[Seg], i: usize) -> Option<usize> {
    let mut j = i;
    let mut assigned = false;
    while let Some(Seg::Other(c)) = segs.get(j) {
        match c {
            '=' | ':' => assigned = true,
            '"' | '\'' | ' ' => {}
            _ => return None,
        }
        j += 1;
        if j - i > 4 {
            return None;
        }
    }
    (assigned && matches!(segs.get(j), Some(Seg::Word(_)))).then_some(j)
}

/// The end of a value starting at `i`: up to whitespace, a quote or a separator.
fn skip_value(segs: &[Seg], mut i: usize) -> usize {
    while let Some(s) = segs.get(i) {
        if let Seg::Other(c) = s
            && (c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '&' | '}' | ')' | ']'))
        {
            break;
        }
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_is_local_only_with_the_release_project() {
        assert_eq!(BugReportSetting::Auto.route(false), BugReportRoute::Github);
        assert_eq!(BugReportSetting::Auto.route(true), BugReportRoute::Local);
        assert_eq!(BugReportSetting::Github.route(true), BugReportRoute::Github);
        assert_eq!(BugReportSetting::Local.route(false), BugReportRoute::Local);
        assert_eq!(BugReportSetting::Off.route(true), BugReportRoute::Off);
    }

    #[test]
    fn titles_are_one_line_and_required() {
        let r = |w: &str| Report { what_happened: w.into(), ..Default::default() };
        assert_eq!(validate("  Crash  ", &r(" details ")), Ok(("Crash".into(), r("details"))));
        assert!(validate("  ", &r("x")).is_err());
        assert!(validate("a\nb", &r("")).is_err());
        assert!(validate(&"x".repeat(201), &r("")).is_err());
        assert!(validate("t", &r(&"x".repeat(20_001))).is_err());
        let split = Report { what_happened: "x".repeat(10_000), expected: "x".repeat(10_000), steps: "x".into() };
        assert!(validate("t", &split).is_err(), "the cap is on the sections together");
    }

    #[test]
    fn redaction_strips_the_token_and_home_paths() {
        let r = Redactions {
            secrets: vec!["github_pat_11ABCDEF0123456789_secretvalue".into()],
            home: Some("/Users/alex".into()),
            hostnames: vec!["alexs-mac-mini".into()],
            ..Default::default()
        };
        let text = "loaded github_pat_11ABCDEF0123456789_secretvalue from keychain\n\
                    opened /Users/alex/work/repo/file.rs\n\
                    another /Users/alice/x and /home/bob/y and C:\\Users\\carol\\z\n\
                    serving on Alexs-Mac-Mini.tail1234.ts.net:443 and box.tail99.ts.net\n\
                    header Authorization: Bearer abc.def-123 and bearer qq.rr\n\
                    {\"apiKey\": \"k-123456\", \"name\": \"ok\"} token=xyz&other=1\n\
                    stray ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa and sk-ant-0123456789abcdefghij\n\
                    sha 0123456789abcdef0123456789abcdef01234567 version 0.1.42 time 12:30:01\n";
        let out = redact(text, &r);
        for leaked in [
            "secretvalue",
            "alex",
            "alice",
            "bob",
            "carol",
            "Alexs-Mac-Mini",
            "tail99",
            "abc.def-123",
            "qq.rr",
            "k-123456",
            "xyz",
            "ghp_a",
            "sk-ant",
            "0123456789abcdef0123",
        ] {
            assert!(!out.contains(leaked), "{leaked} leaked:\n{out}");
        }
        for kept in [
            "~/work/repo/file.rs",
            "/Users/[user]/x",
            "/home/[user]/y",
            "\"name\": \"ok\"",
            "other=1",
            "0.1.42",
            "12:30:01",
        ] {
            assert!(out.contains(kept), "{kept} missing:\n{out}");
        }
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn the_users_own_names_go_as_whole_words() {
        let r = Redactions {
            home: Some("/Users/alex".into()),
            names: vec![
                "Acme Billing".into(),
                "acme-co/billing".into(),
                "billing".into(),
                "ACME".into(),
                "pdt-ACME-12-fix-the-invoice-total".into(),
                "web".into(),
                "main".into(),
                "pdt".into(),
                "x".into(),
            ],
            ..Default::default()
        };
        let text = "project Acme Billing at /Users/alex/work/billing/.worktrees/pdt-ACME-12-fix-the-invoice-total\n\
                    pushed to git@github.com:acme-co/billing.git for ACME-12 on main\n\
                    served web/dist from website pdtd for pdt; billing-old and rebilling stay\n";
        let out = redact(text, &r);
        assert_eq!(
            out,
            "project [name] at ~/work/[name]/.worktrees/[name]\n\
             pushed to git@github.com:[name].git for [name]-12 on main\n\
             served [name]/dist from website pdtd for pdt; billing-old and rebilling stay\n"
        );
    }

    #[test]
    fn windows_home_paths_are_hidden_with_either_slash() {
        let r = Redactions { home: Some("C:\\Users\\Alex".into()), ..Default::default() };
        assert_eq!(redact("C:\\Users\\Alex\\work and C:/Users/Alex/x", &r), "~\\work and ~/x");
    }

    #[test]
    fn the_body_fences_diagnostics_safely() {
        let r = Report { what_happened: "It broke.".into(), ..Default::default() };
        let b = body(&r, Some("log ``` line"), "0.1.42", Some("macos (aarch64)"));
        assert!(b.contains("<details><summary>Diagnostics</summary>"));
        assert!(b.contains("````text\nlog ``` line\n````"));
        assert!(b.contains("0.1.42"));
        assert!(!body(&r, None, "1", None).contains("<details>"));
    }

    #[test]
    fn the_body_has_the_bug_forms_sections() {
        let r = Report { what_happened: "It broke.".into(), steps: "1. Drag a card.".into(), ..Default::default() };
        let b = body(&r, None, "0.1.42", Some("macos (aarch64)"));
        assert!(
            b.starts_with(
                "### What happened\n\nIt broke.\n\n### What you expected\n\n_No response_\n\n\
             ### How to reproduce\n\n1. Drag a card.\n\n### PDT version\n\n0.1.42\n\n### OS\n\nmacos (aarch64)\n\n"
            ),
            "{b}"
        );
        assert!(body(&r, None, "1", None).contains("### OS\n\n_Not shared_\n"));
    }

    /// The GitHub bug form asks for the same sections, and no issue template adds `from-pdt` (triage keys off it:
    /// only reports filed from PDT carry it).
    #[test]
    fn the_github_issue_forms_match_the_report() {
        let bug = include_str!("../../../.github/ISSUE_TEMPLATE/bug.yml");
        let labels: Vec<&str> =
            bug.lines().filter_map(|l| l.trim().strip_prefix("label: ")).map(|l| l.trim_matches('"')).collect();
        assert_eq!(labels, SECTIONS, "bug.yml's field labels, in order");
        let feature = include_str!("../../../.github/ISSUE_TEMPLATE/feature.yml");
        for t in [bug, feature] {
            let labels = t.lines().find_map(|l| l.strip_prefix("labels: ")).expect("a labels line");
            assert!(!labels.contains(crate::github_issue::FROM_PDT), "an issue template adds {labels}");
        }
    }

    #[test]
    fn diagnostics_render_and_tail() {
        assert_eq!(tail("a\nb\nc\n", 2), vec!["b", "c"]);
        assert_eq!(tail("a", 5), vec!["a"]);
        let d = Diagnostics {
            version: "0.1.42".into(),
            profile: "stable".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            supervised: true,
            update: "idle".into(),
            token: "valid".into(),
            log_tail: vec!["INFO started".into()],
            ..Default::default()
        };
        let t = d.render();
        assert!(t.contains("PDT version: 0.1.42") && t.contains("Supervised by pdt-host: yes"));
        assert!(t.ends_with("INFO started\n"));
    }
}
