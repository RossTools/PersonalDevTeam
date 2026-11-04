//! Naming a ticket's branch from a template (PDT-594): a project's own convention, such as `{type}/{jira}-{slug}`,
//! from its Settings or the `[branch]` table of `.pdt/project.toml`. Pure: pdt-api gathers the ticket's facts and
//! checks which names are taken.
//!
//! A template is text with parts in braces. `{a|b}` takes the first of the parts that has a value; a part with none
//! is dropped with its separator, so no name has doubled, leading or trailing `-`, `_`, `.` or `/`.

use crate::brand::SLUG;
use crate::github_issue::GITHUB;
use crate::ticket::{TicketSummary, slug};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// The longest branch name a template makes; the slug is cut first.
pub const MAX_BRANCH: usize = 100;
/// The longest template.
pub const MAX_TEMPLATE: usize = 200;
/// `{type}` for a ticket none of whose labels is in the types map.
pub const DEFAULT_TYPE: &str = "feat";
/// The characters that separate words in a name: runs of them collapse to one.
const SEPARATORS: &str = "-_./";

/// The template a project without one uses: `pdt-{key}-{slug}`, the names PDT has always made.
pub fn default_template() -> String {
    format!("{SLUG}-{{key}}-{{slug}}")
}

/// The labels → `{type}` map every project starts from; its own `types` add to it (and win).
pub fn default_types() -> BTreeMap<String, String> {
    BTreeMap::from([("bug".to_string(), "fix".to_string())])
}

/// A project's branch naming as written (Settings, or `[branch]` in `.pdt/project.toml`): no template is the default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BranchSettings {
    #[serde(default)]
    pub template: Option<String>,
    /// Ticket label → `{type}` (`bug = "fix"`), over the defaults.
    #[serde(default)]
    pub types: BTreeMap<String, String>,
}

/// What a ticket's branch name is made from, as it is when work starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchFacts {
    /// `PDT`.
    pub project_key: String,
    pub number: u32,
    pub title: String,
    pub labels: Vec<String>,
    /// Its linked issues: (tracker, key), as `ticket_links_external` has them (`github`, `owner/repo#12`).
    pub links: Vec<(String, String)>,
    /// The parent's key, for a part.
    pub parent: Option<String>,
}

impl BranchFacts {
    pub fn of(t: &TicketSummary) -> Self {
        let project_key = t.key.rsplit_once('-').map(|(k, _)| k).unwrap_or(&t.key).to_string();
        BranchFacts {
            project_key,
            number: t.number,
            title: t.title.clone(),
            labels: t.labels.clone(),
            links: t.external.iter().map(|l| (l.system.clone(), l.key.clone())).collect(),
            parent: t.parent.as_ref().map(|p| p.key.clone()),
        }
    }

    fn link(&self, system: &str) -> Option<&str> {
        self.links.iter().find(|(s, _)| s == system).map(|(_, k)| k.as_str())
    }

    /// The linked GitHub issue's repository and number.
    fn github_issue(&self) -> Option<(&str, &str)> {
        self.link(GITHUB)?.rsplit_once('#')
    }
}

/// A part a template can use, for Settings' list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PartInfo {
    /// `jira`, written `{jira}`.
    pub name: String,
    pub description: String,
}

/// A built-in part's value for a ticket, given the `{type}` map.
type PartFn = fn(&BranchFacts, &BTreeMap<String, String>) -> Option<String>;

/// Where a part's value comes from.
#[derive(Clone, Copy)]
enum Source {
    Builtin(PartFn),
    /// The key of the ticket's linked issue in this tracker (an integration's id).
    Tracker(&'static str),
}

struct Part {
    name: &'static str,
    description: String,
    source: Source,
}

/// Every part, in Settings' order: the built-ins, then one per tracker that names a part in pdt-core
/// `integration::KNOWN` (`branch_part`). A new tracker part is a `branch_part` there.
fn parts() -> Vec<Part> {
    let builtin =
        |name, description: &str, f: PartFn| Part { name, description: description.into(), source: Source::Builtin(f) };
    let mut v = vec![
        builtin("key", "The ticket's key: PDT-48", |f, _| Some(format!("{}-{}", f.project_key, f.number))),
        builtin("project", "The project's key: PDT", |f, _| Some(f.project_key.clone())),
        builtin("number", "The ticket's number: 48", |f, _| Some(f.number.to_string())),
        builtin("slug", "The title, shortened: fix-the-login-crash", |f, _| Some(slug(&f.title))),
        builtin("type", "From the labels through the types map: fix for bug, otherwise feat", |f, types| {
            Some(type_of(&f.labels, types))
        }),
        builtin("issue", "The linked GitHub issue's number: 123", |f, _| f.github_issue().map(|(_, n)| n.into())),
        builtin("issue_repo", "The linked GitHub issue's repository: owner-repo", |f, _| {
            f.github_issue().map(|(r, _)| r.into())
        }),
        builtin("parent", "The parent ticket's key, for a part: PDT-40", |f, _| f.parent.clone()),
    ];
    for k in crate::integration::KNOWN {
        if let Some(name) = k.branch_part {
            let description = format!("The linked {} issue's key: ABC-123", k.label);
            v.push(Part { name, description, source: Source::Tracker(k.id) });
        }
    }
    v
}

/// The parts a template can use, for Settings.
pub fn part_list() -> Vec<PartInfo> {
    parts().into_iter().map(|p| PartInfo { name: p.name.into(), description: p.description }).collect()
}

/// `{type}`: the first of the ticket's labels that `types` (over the defaults) maps, else `feat`.
fn type_of(labels: &[String], types: &BTreeMap<String, String>) -> String {
    let defaults = default_types();
    labels
        .iter()
        .find_map(|l| {
            let l = l.to_ascii_lowercase();
            types.get(&l).or_else(|| defaults.get(&l)).cloned()
        })
        .unwrap_or_else(|| DEFAULT_TYPE.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Text(String),
    /// The parts of `{a|b|c}`, by index in [`parts`]: the first with a value.
    Parts(Vec<usize>),
}

/// A parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    tokens: Vec<Token>,
}

/// Whether `c` may be written in a template's text: letters, digits, `.`, `_`, `-` and `/`.
fn text_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || SEPARATORS.contains(c)
}

impl Template {
    pub fn parse(text: &str) -> Result<Template, String> {
        if text.trim().is_empty() {
            return Err("the template is empty".into());
        }
        if text.len() > MAX_TEMPLATE {
            return Err(format!("the template is longer than {MAX_TEMPLATE} characters"));
        }
        let known = parts();
        let mut tokens = vec![];
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix('{') {
                let Some((inner, tail)) = after.split_once('}') else {
                    return Err(format!("a {{ is not closed in \"{text}\""));
                };
                let mut alts = vec![];
                for name in inner.split('|').map(str::trim) {
                    match known.iter().position(|p| p.name == name) {
                        Some(i) => alts.push(i),
                        None if name.is_empty() => return Err(format!("\"{{{inner}}}\" names no part")),
                        None => {
                            let names: Vec<String> = known.iter().map(|p| format!("{{{}}}", p.name)).collect();
                            return Err(format!("{{{name}}} is not a part (the parts are {})", names.join(", ")));
                        }
                    }
                }
                tokens.push(Token::Parts(alts));
                rest = tail;
            } else {
                let end = rest.find('{').unwrap_or(rest.len());
                let (lit, tail) = rest.split_at(end);
                if let Some(c) = lit.chars().find(|c| !text_char(*c)) {
                    return Err(format!(
                        "\"{c}\" can't be in a branch name template: write letters, digits, . _ - / and parts in {{}}"
                    ));
                }
                tokens.push(Token::Text(lit.to_string()));
                rest = tail;
            }
        }
        Ok(Template { tokens })
    }

    /// The name with `slug` as the slug (cut to fit), before the length cap and git's rules.
    fn render(&self, facts: &BranchFacts, types: &BTreeMap<String, String>, slug: &str, known: &[Part]) -> String {
        let mut out = String::new();
        for t in &self.tokens {
            match t {
                Token::Text(s) => out.push_str(s),
                Token::Parts(alts) => {
                    let value = alts.iter().find_map(|i| {
                        let v = match (known[*i].name, known[*i].source) {
                            ("slug", _) => Some(slug.to_string()),
                            (_, Source::Builtin(f)) => f(facts, types),
                            (_, Source::Tracker(id)) => facts.link(id).map(str::to_string),
                        };
                        v.map(|v| clean_value(&v)).filter(|v| !v.is_empty())
                    });
                    out.push_str(&value.unwrap_or_default());
                }
            }
        }
        tidy(&out)
    }
}

/// A part's value as it goes in a name: only letters, digits, `.`, `_` and `-` (anything else, `/` too, is a `-`),
/// tidied.
fn clean_value(v: &str) -> String {
    let mapped: String =
        v.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
    tidy(&mapped)
}

/// Each run of separators becomes one (a `/` when the run has one, else its first), and none lead or trail.
fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |out: &mut String, run: &mut String| {
        if !run.is_empty() {
            if !out.is_empty() {
                out.push(if run.contains('/') { '/' } else { run.chars().next().unwrap_or('-') });
            }
            run.clear();
        }
    };
    for c in s.chars() {
        if SEPARATORS.contains(c) {
            run.push(c);
        } else {
            flush(&mut out, &mut run);
            out.push(c);
        }
    }
    out
}

/// Whether git takes `name` as a branch name (`git check-ref-format --branch`'s rules) and it is at most
/// [`MAX_BRANCH`] long.
pub fn valid_branch_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_BRANCH || name == "@" || name.starts_with('-') {
        return false;
    }
    if name.contains("..") || name.contains("@{") || name.ends_with('.') || name.contains("//") {
        return false;
    }
    if name.chars().any(|c| c.is_ascii_control() || c == ' ' || "~^:?*[\\".contains(c) || c == '\u{7f}') {
        return false;
    }
    name.split('/').all(|c| !c.is_empty() && !c.starts_with('.') && !c.ends_with(".lock"))
}

/// A project's branch naming, checked: its template and `{type}` map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchNaming {
    template: Template,
    text: String,
    types: BTreeMap<String, String>,
}

impl Default for BranchNaming {
    fn default() -> Self {
        BranchNaming::new(&BranchSettings::default()).expect("the default template is valid")
    }
}

impl BranchNaming {
    /// Checks a template and types: the template parses, and gives a valid name for a ticket with every part and for
    /// one with only the parts every ticket has (no links, no parent, a title with no slug).
    pub fn new(s: &BranchSettings) -> Result<BranchNaming, String> {
        let text = s.template.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(String::from);
        let text = text.unwrap_or_else(default_template);
        let template = Template::parse(&text)?;
        let mut types = BTreeMap::new();
        for (label, ty) in &s.types {
            let (label, ty) = (label.trim().to_ascii_lowercase(), ty.trim());
            if label.is_empty() {
                return Err("a type needs a label".into());
            }
            if ty.is_empty() || !ty.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
                return Err(format!(
                    "\"{ty}\" (for the label {label}) can't be a type: use letters, digits, . _ and -"
                ));
            }
            types.insert(label, ty.to_string());
        }
        let naming = BranchNaming { template, text, types };
        for f in [sample(true), BranchFacts { title: String::new(), ..sample(false) }] {
            let name = naming.render_capped(&f);
            if name.is_empty() {
                return Err(format!(
                    "\"{}\" gives an empty name for a ticket without links: add {{key}} or {{slug}}, or a fallback such as {{jira|key}}",
                    naming.text
                ));
            }
            if !valid_branch_name(&name) {
                return Err(format!("\"{}\" gives \"{name}\", which git does not take as a branch name", naming.text));
            }
        }
        Ok(naming)
    }

    pub fn template(&self) -> &str {
        &self.text
    }

    /// The `{type}` map, the defaults included.
    pub fn types(&self) -> BTreeMap<String, String> {
        let mut all = default_types();
        all.extend(self.types.clone());
        all
    }

    /// Whether names may have a `/`, which some tools handle badly.
    pub fn has_slash(&self) -> bool {
        self.template.tokens.iter().any(|t| matches!(t, Token::Text(s) if s.contains('/')))
    }

    fn render_capped(&self, facts: &BranchFacts) -> String {
        let known = parts();
        let full = slug(&facts.title);
        let mut name = self.template.render(facts, &self.types, &full, &known);
        if name.len() > MAX_BRANCH {
            // The slug goes first, a word at a time.
            let over = name.len() - MAX_BRANCH;
            let cut = cut_slug(&full, full.len().saturating_sub(over));
            name = self.template.render(facts, &self.types, &cut, &known);
        }
        if name.len() > MAX_BRANCH {
            name = tidy(&name[..MAX_BRANCH]);
        }
        name
    }

    /// The branch name for a ticket; the default template's when this one gives no valid name for it.
    pub fn name(&self, facts: &BranchFacts) -> String {
        let name = self.render_capped(facts);
        if valid_branch_name(&name) { name } else { BranchNaming::default().render_capped(facts) }
    }
}

/// `slug` cut to at most `max` characters at a dash where it can be.
fn cut_slug(slug: &str, max: usize) -> String {
    if slug.len() <= max {
        return slug.to_string();
    }
    let head = &slug[..max];
    match head.rfind('-') {
        Some(i) if slug.as_bytes().get(max) != Some(&b'-') => head[..i].to_string(),
        _ => head.trim_end_matches('-').to_string(),
    }
}

/// A sample ticket for checking templates and Settings' preview: a bug, with every link a part can use when `links`.
pub fn sample(links: bool) -> BranchFacts {
    let mut f = BranchFacts {
        project_key: "PDT".into(),
        number: 48,
        title: "Fix the login crash".into(),
        labels: vec!["bug".into()],
        links: vec![],
        parent: None,
    };
    if links {
        f.links.push((GITHUB.into(), "acme/app#123".into()));
        for k in crate::integration::KNOWN {
            if k.branch_part.is_some() {
                f.links.push((k.id.into(), "ABC-123".into()));
            }
        }
    }
    f
}

/// `name`, or the first of `name-2`, `name-3`, … that is not `taken` (each cut to fit [`MAX_BRANCH`]).
pub fn unique(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let mut n = 2u32;
    loop {
        let suffix = format!("-{n}");
        let head = tidy(&name[..name.len().min(MAX_BRANCH - suffix.len())]);
        let candidate = format!("{head}{suffix}");
        if !taken(&candidate) || n >= 1000 {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naming(t: &str) -> BranchNaming {
        BranchNaming::new(&BranchSettings { template: Some(t.into()), types: BTreeMap::new() }).unwrap()
    }

    fn facts(title: &str) -> BranchFacts {
        BranchFacts { project_key: "PDT".into(), number: 48, title: title.into(), ..Default::default() }
    }

    fn with_links(mut f: BranchFacts) -> BranchFacts {
        f.links = vec![("github".into(), "acme/app#123".into()), ("jira".into(), "ABC-123".into())];
        f
    }

    #[test]
    fn the_default_template_names_branches_as_pdt_always_has() {
        let d = BranchNaming::default();
        assert_eq!(d.template(), "pdt-{key}-{slug}");
        for (key, n, title) in [
            ("PDT", 42, "Fix the Login bug!"),
            ("PDT", 7, "日本語"),
            ("AB", 1, "  "),
            ("PDT", 3, "a very long ticket title that keeps going well past forty characters"),
        ] {
            let f = BranchFacts { project_key: key.into(), number: n, title: title.into(), ..Default::default() };
            assert_eq!(d.name(&f), crate::start::attempt_branch(key, n, title), "{title}");
        }
        assert_eq!(
            BranchNaming::new(&BranchSettings { template: Some("  ".into()), ..Default::default() }).unwrap(),
            d
        );
    }

    #[test]
    fn every_part_has_its_value() {
        let mut f = with_links(facts("Fix the login crash"));
        f.labels = vec!["bug".into()];
        f.parent = Some("PDT-40".into());
        let cases = [
            ("{key}", "PDT-48"),
            ("{project}", "PDT"),
            ("{number}", "48"),
            ("{slug}", "fix-the-login-crash"),
            ("{type}", "fix"),
            ("{issue}", "123"),
            ("{issue_repo}", "acme-app"),
            ("{jira}", "ABC-123"),
            ("{parent}", "PDT-40"),
        ];
        for (t, want) in cases {
            let n = BranchNaming::new(&BranchSettings { template: Some(format!("{t}-{{key}}")), ..Default::default() })
                .unwrap();
            let got = n.name(&f);
            assert_eq!(got.strip_suffix("-PDT-48").unwrap_or(&got), want, "{t}");
        }
        let names: Vec<String> = part_list().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["key", "project", "number", "slug", "type", "issue", "issue_repo", "parent", "jira"]);
    }

    #[test]
    fn type_comes_from_the_labels_through_the_map() {
        let t = |labels: &[&str], types: &[(&str, &str)]| {
            let types = types.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
            let n = BranchNaming::new(&BranchSettings { template: Some("{type}/{key}".into()), types }).unwrap();
            let f = BranchFacts { labels: labels.iter().map(|s| s.to_string()).collect(), ..facts("x") };
            n.name(&f)
        };
        assert_eq!(t(&[], &[]), "feat/PDT-48");
        assert_eq!(t(&["Bug"], &[]), "fix/PDT-48");
        assert_eq!(t(&["ui", "docs"], &[("docs", "docs")]), "docs/PDT-48");
        assert_eq!(t(&["bug"], &[("bug", "bugfix")]), "bugfix/PDT-48", "a project's types win over the defaults");
        let bad = BranchSettings { template: None, types: BTreeMap::from([("bug".into(), "a b".into())]) };
        assert!(BranchNaming::new(&bad).unwrap_err().contains("can't be a type"));
    }

    #[test]
    fn a_fallback_takes_the_first_part_with_a_value() {
        let n = naming("{jira|issue|key}-{slug}");
        assert_eq!(n.name(&with_links(facts("Login"))), "ABC-123-login");
        let mut gh = facts("Login");
        gh.links = vec![("github".into(), "acme/app#9".into())];
        assert_eq!(n.name(&gh), "9-login");
        assert_eq!(n.name(&facts("Login")), "PDT-48-login");
    }

    #[test]
    fn an_empty_part_goes_with_its_separator() {
        let n = naming("{type}/{jira}-{slug}");
        let mut f = with_links(facts("Login crash"));
        f.labels = vec!["bug".into()];
        assert_eq!(n.name(&f), "fix/ABC-123-login-crash");
        f.links.clear();
        assert_eq!(n.name(&f), "fix/login-crash");
        assert_eq!(naming("{jira}_{key}__{slug}").name(&facts("日本語")), "PDT-48");
        assert_eq!(naming("x/{parent}/{key}.{slug}").name(&facts("")), "x/PDT-48");
        assert_eq!(naming("feature/{issue}-{key}").name(&facts("a")), "feature/PDT-48");
    }

    #[test]
    fn bad_templates_are_refused() {
        let err = |t: &str| {
            BranchNaming::new(&BranchSettings { template: Some(t.into()), ..Default::default() }).unwrap_err()
        };
        assert!(err("{key").contains("not closed"));
        assert!(err("{nope}-{key}").contains("{nope} is not a part"));
        assert!(err("{}-{key}").contains("names no part"));
        assert!(err("a b/{key}").contains("can't be in a branch name"));
        assert!(err("{key}~1").contains("can't be in"));
        assert!(err("{jira}").contains("empty name"), "a ticket without a Jira link would get no name");
        assert!(err("{slug}").contains("empty name"), "a title with no slug would give no name");
        assert!(err("x.lock/{key}").contains("git does not take"));
        assert!(err(&"a".repeat(MAX_TEMPLATE + 1)).contains("longer"));
        assert!(
            BranchNaming::new(&BranchSettings { template: Some("{slug|key}".into()), ..Default::default() }).is_ok()
        );
    }

    #[test]
    fn names_follow_gits_rules() {
        for ok in ["main", "fix/ABC-1-x", "a.b", "pdt-PDT-1"] {
            assert!(valid_branch_name(ok), "{ok}");
        }
        for bad in [
            "", "@", "-x", "a..b", "a@{b", "a.", "a//b", "a b", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b", "a\\b", ".a",
            "a/.b", "a.lock", "a.lock/b", "a/",
        ] {
            assert!(!valid_branch_name(bad), "{bad}");
        }
        assert!(!valid_branch_name(&"a".repeat(MAX_BRANCH + 1)));
        // Part values never break them: a slash in a key becomes a dash.
        let mut f = facts("x");
        f.links = vec![("jira".into(), "AB/C..1".into())];
        assert_eq!(naming("{jira}-{key}").name(&f), "AB-C.1-PDT-48");
    }

    #[test]
    fn long_names_cut_the_slug_first() {
        let n = naming("feature/{project}/{key}-{slug}-end");
        let title = "a long title with many words that go on and on and on past every limit there is today";
        let name = n.name(&BranchFacts { project_key: "PROJECTKEY".repeat(4), ..facts(title) });
        assert!(name.len() <= MAX_BRANCH, "{name}");
        assert!(name.ends_with("-end"), "the slug is cut, not the end: {name}");
        assert!(valid_branch_name(&name));
        // With no slug to cut, the name is cut at the cap.
        let long = naming(&format!("{}-{{key}}", "x".repeat(150)));
        let name = long.name(&facts("t"));
        assert_eq!(name.len(), MAX_BRANCH);
    }

    #[test]
    fn taken_names_get_a_number() {
        let taken = ["fix/login", "fix/login-2"];
        assert_eq!(unique("fix/login", |n| taken.contains(&n)), "fix/login-3");
        assert_eq!(unique("fix/other", |n| taken.contains(&n)), "fix/other");
        let long = "a".repeat(MAX_BRANCH);
        let got = unique(&long, |n| n == long);
        assert_eq!(got.len(), MAX_BRANCH);
        assert!(got.ends_with("-2"));
    }

    #[test]
    fn slashes_are_noticed() {
        assert!(naming("{type}/{key}").has_slash());
        assert!(!BranchNaming::default().has_slash());
    }
}
