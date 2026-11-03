//! Review: a worktree's changes, line comments, the
//! feedback message sent to the coder session, and merging.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// One file a worktree's branch changed (committed or not) since its merge base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangedFile {
    /// Checkout-relative, `/`-separated (a deleted file's old path).
    pub path: String,
    /// The name before a rename.
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    /// A new file git does not track yet.
    pub untracked: bool,
    pub binary: bool,
    pub added: u32,
    pub removed: u32,
    /// Changes whenever either side of the file does ("Viewed" resets then).
    pub version: String,
    /// Made by a tool, not written by hand: marked `linguist-generated` in `.gitattributes`, or a package manager's
    /// lockfile ([`is_lockfile`]). The Changes view starts it collapsed.
    pub generated: bool,
    /// Not committed yet (staged, unstaged or untracked).
    pub uncommitted: bool,
}

/// Lockfiles package managers write (`Cargo.lock`, `yarn.lock`, `package-lock.json`, `pnpm-lock.yaml`…), by name.
pub fn is_lockfile(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".lock")
        || matches!(
            name,
            "package-lock.json"
                | "npm-shrinkwrap.json"
                | "pnpm-lock.yaml"
                | "bun.lockb"
                | "go.sum"
                | "packages.lock.json"
        )
}

/// `GET /api/worktrees/{id}/changes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Changes {
    pub worktree_id: String,
    pub branch: String,
    /// The base branch the changes are measured against.
    pub base: String,
    /// The commit they are measured from.
    pub merge_base: String,
    pub head: String,
    /// Uncommitted changes (untracked files included).
    pub dirty: bool,
    pub files: Vec<ChangedFile>,
    /// The attempt working in this worktree (its active one, else the newest), and its ticket.
    pub attempt_id: Option<String>,
    pub ticket_id: Option<String>,
    /// The attempt is active (Approve and Merge are offered).
    pub attempt_active: bool,
    /// The user approved the attempt at the current head.
    pub approved: bool,
}

/// `GET /api/worktrees/{id}/changes/file`: a file at the merge base and in the working tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileSides {
    pub path: String,
    /// None for an added file (or when not served: binary, too large).
    pub old: Option<String>,
    /// None for a deleted file (or when not served).
    pub new: Option<String>,
    pub binary: bool,
    /// A side is over [`MAX_SIDE_BYTES`].
    pub too_large: bool,
}

/// The largest side of a file the diff serves.
pub const MAX_SIDE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CommentSide {
    /// Lines the change removed (numbered as in the merge base).
    Old,
    /// Lines in the working tree.
    New,
}

impl CommentSide {
    pub fn as_str(self) -> &'static str {
        match self {
            CommentSide::Old => "old",
            CommentSide::New => "new",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "old" => Some(CommentSide::Old),
            "new" => Some(CommentSide::New),
            _ => None,
        }
    }
}

/// A line comment on a worktree's changes. A root starts a thread; a reply has `parent_id` (its root) and the root's
/// anchor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewComment {
    pub id: String,
    pub worktree_id: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub side: CommentSide,
    /// 1-based first line on `side`.
    pub line: u32,
    /// Last line (= `line` for one line).
    pub end_line: u32,
    /// The anchored lines when the comment was made, to find them again after the file changes.
    pub line_text: String,
    pub body: String,
    /// `user`, or the persona name of the agent session that wrote it (e.g. `Reviewer`).
    pub author: String,
    /// None while a draft.
    #[ts(type = "number | null")]
    pub sent_at: Option<i64>,
    /// The session it was sent to.
    pub session_id: Option<String>,
    /// Roots only.
    #[ts(type = "number | null")]
    pub resolved_at: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// Longest comment body, in bytes.
pub const MAX_COMMENT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NewReviewComment {
    pub path: String,
    pub side: CommentSide,
    pub line: u32,
    #[serde(default)]
    #[ts(optional)]
    pub end_line: Option<u32>,
    pub line_text: String,
    pub body: String,
    /// Reply to this comment's thread.
    #[serde(default)]
    #[ts(optional)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewCommentPatch {
    /// Drafts only.
    #[serde(default)]
    #[ts(optional)]
    pub body: Option<String>,
    /// Roots only.
    #[serde(default)]
    #[ts(optional)]
    pub resolved: Option<bool>,
}

/// `POST /api/worktrees/{id}/feedback`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FeedbackResult {
    pub session_id: String,
    /// Comments sent.
    pub sent: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeMethod {
    /// A merge commit on the local base branch.
    Local,
    /// The forge merges the attempt's open change request (its auto-merge, or the base branch's merge queue). Was
    /// `github` (still read).
    #[serde(alias = "github")]
    Forge,
}

/// Why Merge can't go ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeProblem {
    /// The attempt is not active.
    Inactive,
    /// The worktree has uncommitted changes.
    Dirty,
    /// The merge would conflict (`conflicts`).
    Conflict,
    /// The checkout that has the base branch checked out has uncommitted changes.
    BaseDirty,
    /// The branch has no commits the base lacks.
    Nothing,
    /// The forge says the PR can't merge (conflicts) or it is a draft.
    PrNotMergeable,
    /// Auto-merge is already on, or the PR is in the merge queue.
    PrQueued,
    /// The project merges through pull requests and the attempt has no open one yet.
    NoPr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MergePr {
    pub number: u32,
    pub url: String,
    /// The forge's name (`GitHub`).
    pub forge: String,
    /// The base branch has a merge queue (it picks the method).
    pub queue: bool,
    /// What the forge calls its merge queue (`merge queue`, `merge train`).
    pub queue_name: String,
    /// A change request on the local forge (PDT-178): PDT merges it here once its CI passes.
    #[serde(default)]
    pub local: bool,
    /// The forge can hold the merge until the checks pass (GitHub's auto-merge, allowed in the repository's
    /// settings). Without it (PDT-297) PDT merges the pull request now when CI passed, or once it passes.
    #[serde(default)]
    pub auto_merge: bool,
}

/// `GET /api/attempts/{id}/merge`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MergePreview {
    pub method: MergeMethod,
    pub branch: String,
    pub base: String,
    /// Commits on the branch the base lacks.
    pub commits: u32,
    pub problem: Option<MergeProblem>,
    /// With `conflict`: the files.
    pub conflicts: Vec<String>,
    /// With `base_dirty`: the checkout.
    pub base_checkout: Option<String>,
    pub pr: Option<MergePr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeOutcome {
    /// Merged locally; the attempt ended as merged.
    Merged,
    /// Set to merge: auto-merge is on or it is queued (the forge merges once the checks pass), or PDT merges it once
    /// CI passes where the forge can't hold it (PDT-297). The merge check then ends the attempt.
    Queued,
}

/// `POST /api/attempts/{id}/merge`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MergeResult {
    pub outcome: MergeOutcome,
    pub commit: Option<String>,
}

/// How a local merge lands on the base branch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MergeStyle {
    /// A merge commit (`--no-ff`): the branch's commits stay in the history.
    #[default]
    Merge,
    /// One new commit on the base with the branch's changes.
    Squash,
}

/// `POST /api/attempts/{id}/merge` (an empty body is a merge commit). `style` is for a local merge only; a pull
/// request merges the way the forge's settings say.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MergeRequest {
    #[serde(default)]
    #[ts(optional)]
    pub style: Option<MergeStyle>,
    /// A pull request only: once the forge merges it, move the ticket to Done and remove the worktree and the workspace
    /// Start made (PDT-75). A local merge finishes in the web right away. Absent: what an earlier merge of the
    /// attempt asked stays (none: don't finish).
    #[serde(default)]
    #[ts(optional)]
    pub finish_on_merge: Option<bool>,
    /// With `finish_on_merge`: delete the branch with the worktree.
    #[serde(default)]
    #[ts(optional)]
    pub delete_branch: Option<bool>,
    /// `user`: a person merges (PDT's Merge… sends it). Absent or `agent`: an automated merge, which the project's
    /// policy may refuse.
    #[serde(default)]
    #[ts(optional)]
    pub by: Option<String>,
}

/// The local merge commit's message.
pub fn merge_message(branch: &str, title: &str) -> String {
    format!("Merge {branch}: {title}")
}

/// The local squash commit's message (as GitHub's squash: the ticket, then where it came from).
pub fn squash_message(key: &str, title: &str, branch: &str) -> String {
    format!("{key}: {title}\n\nSquashed from {branch}.")
}

/// One comment as the feedback message shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackItem {
    pub path: String,
    pub side: CommentSide,
    pub line: u32,
    pub end_line: u32,
    pub line_text: String,
    pub body: String,
    /// For a reply: the body of its thread's first comment.
    pub reply_to: Option<String>,
}

/// Quoted lines per comment before `> …`.
const QUOTE_LINES: usize = 6;
/// How much of a thread's first comment a reply quotes.
const REPLY_QUOTE_CHARS: usize = 80;
/// The whole message's limit.
pub const MAX_FEEDBACK_BYTES: usize = 32 * 1024;

fn cut(s: &str, chars: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= chars {
        one_line
    } else {
        format!("{}…", one_line.chars().take(chars).collect::<String>().trim_end())
    }
}

fn indent(text: &str) -> String {
    text.lines().map(|l| if l.is_empty() { String::new() } else { format!("   {l}") }).collect::<Vec<_>>().join("\n")
}

fn render(n: usize, item: &FeedbackItem) -> String {
    let range = if item.end_line > item.line {
        format!("{}:{}-{}", item.path, item.line, item.end_line)
    } else {
        format!("{}:{}", item.path, item.line)
    };
    let removed = if item.side == CommentSide::Old { " (removed lines)" } else { "" };
    let mut out = match &item.reply_to {
        Some(root) => format!("{n}. Reply on {range}{removed} (\"{}\"):\n", cut(root, REPLY_QUOTE_CHARS)),
        None => {
            let mut s = format!("{n}. {range}{removed}\n");
            let lines: Vec<&str> = item.line_text.lines().collect();
            for l in lines.iter().take(QUOTE_LINES) {
                s.push_str(&format!("   > {l}\n").replace("   > \n", "   >\n"));
            }
            if lines.len() > QUOTE_LINES {
                s.push_str("   > …\n");
            }
            s
        }
    };
    out.push_str(&indent(item.body.trim()));
    out
}

/// The one message Send feedback puts into the coder session: a heading, then each comment in file and line order
/// (a reply right after its thread's earlier comments, as given), with the lines it is about quoted. Over
/// [`MAX_FEEDBACK_BYTES`], the last comments are left out and counted.
pub fn feedback_message(branch: &str, items: &[FeedbackItem]) -> String {
    let mut sorted: Vec<&FeedbackItem> = items.iter().collect();
    sorted.sort_by(|a, b| (a.path.as_str(), a.line).cmp(&(b.path.as_str(), b.line)));
    let count = if sorted.len() == 1 { "1 comment".to_string() } else { format!("{} comments", sorted.len()) };
    let mut out =
        format!("Review feedback on {branch} ({count}). Address each one, then say briefly what you changed.\n");
    for (i, item) in sorted.iter().enumerate() {
        let part = format!("\n{}\n", render(i + 1, item));
        let left = sorted.len() - i;
        let tail = format!("\n…and {left} more comment{}, see the Changes view.\n", if left == 1 { "" } else { "s" });
        if out.len() + part.len() + tail.len() > MAX_FEEDBACK_BYTES {
            out.push_str(&tail);
            return out.trim_end().to_string();
        }
        out.push_str(&part);
    }
    out.trim_end().to_string()
}

/// A code fence for `text`: longer than any run of backticks in it.
fn fence(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

/// What the ticket timeline keeps of sent comments (the `feedback` event's Markdown body; the comments themselves go
/// with the worktree): each comment in the message's order, its place, the lines it is about and its text. Over
/// [`MAX_FEEDBACK_BYTES`], the last ones are counted instead.
pub fn feedback_record(items: &[FeedbackItem]) -> String {
    let mut sorted: Vec<&FeedbackItem> = items.iter().collect();
    sorted.sort_by(|a, b| (a.path.as_str(), a.line).cmp(&(b.path.as_str(), b.line)));
    let mut parts: Vec<String> = Vec::new();
    let mut size = 0;
    for (i, item) in sorted.iter().enumerate() {
        let range = if item.end_line > item.line {
            format!("{}:{}-{}", item.path, item.line, item.end_line)
        } else {
            format!("{}:{}", item.path, item.line)
        };
        let removed = if item.side == CommentSide::Old { " (removed lines)" } else { "" };
        let mut part = match &item.reply_to {
            Some(root) => format!("**Reply on `{range}`**{removed} (\"{}\")\n\n", cut(root, REPLY_QUOTE_CHARS)),
            None => {
                let lines: Vec<&str> = item.line_text.lines().collect();
                let mut quoted = lines.iter().take(QUOTE_LINES).copied().collect::<Vec<_>>().join("\n");
                if lines.len() > QUOTE_LINES {
                    quoted.push_str("\n…");
                }
                let f = fence(&quoted);
                format!("**`{range}`**{removed}\n\n{f}\n{quoted}\n{f}\n\n")
            }
        };
        part.push_str(item.body.trim());
        let left = sorted.len() - i;
        let tail = format!("…and {left} more comment{}.", if left == 1 { "" } else { "s" });
        if size + part.len() + tail.len() + 2 > MAX_FEEDBACK_BYTES {
            parts.push(tail);
            break;
        }
        size += part.len() + 2;
        parts.push(part);
    }
    parts.join("\n\n")
}

/// The message Send to agent puts into the coder session when the merge conflicts.
pub fn conflict_message(branch: &str, base: &str, files: &[String]) -> String {
    let list = files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n");
    format!(
        "Merging {branch} into {base} conflicts in these files:\n{list}\n\nMerge {base} into {branch}, resolve the \
         conflicts, run the checks and commit."
    )
}

/// The message Ask the agent puts into the coder session when the project merges through pull requests and the
/// branch has none yet.
pub fn open_pr_message(branch: &str, base: &str) -> String {
    format!(
        "{branch} is ready to merge, and this project merges through pull requests. Commit anything left, push \
         {branch} and open a pull request against {base} (`change_request_open`)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_common_lockfiles_by_name() {
        for p in [
            "Cargo.lock",
            "web/yarn.lock",
            "Gemfile.lock",
            "package-lock.json",
            "a/pnpm-lock.yaml",
            "go.sum",
            "uv.lock",
        ] {
            assert!(is_lockfile(p), "{p}");
        }
        for p in ["src/lock.rs", "lockfile.md", "package.json", "pnpm-lock.yaml.bak", "docs/locks/a.txt"] {
            assert!(!is_lockfile(p), "{p}");
        }
    }

    fn item(path: &str, line: u32, end: u32, text: &str, body: &str) -> FeedbackItem {
        FeedbackItem {
            path: path.into(),
            side: CommentSide::New,
            line,
            end_line: end,
            line_text: text.into(),
            body: body.into(),
            reply_to: None,
        }
    }

    #[test]
    fn the_message_lists_comments_in_file_and_line_order_with_their_lines() {
        let mut removed = item("web/x.tsx", 40, 40, "const a = 1;", "Why was this removed?");
        removed.side = CommentSide::Old;
        let mut reply = item("docs/a.md", 3, 3, "see the repo path", "Done? It still says the repo path.");
        reply.reply_to = Some("Use the API path,\nnot the repo path, everywhere in this guide please thanks".into());
        let msg = feedback_message(
            "pdt/PDT-3-x",
            &[
                removed,
                item("crates/a.rs", 19, 21, "pub fn a(\n    x: u32,\n) {", "Name the fallback.\n\nAnd test it."),
                reply,
            ],
        );
        assert_eq!(
            msg,
            "Review feedback on pdt/PDT-3-x (3 comments). Address each one, then say briefly what you changed.\n\
             \n\
             1. crates/a.rs:19-21\n   > pub fn a(\n   >     x: u32,\n   > ) {\n   Name the fallback.\n\n   And test it.\n\
             \n\
             2. Reply on docs/a.md:3 (\"Use the API path, not the repo path, everywhere in this guide please thanks\"):\n   \
             Done? It still says the repo path.\n\
             \n\
             3. web/x.tsx:40 (removed lines)\n   > const a = 1;\n   Why was this removed?"
        );
    }

    #[test]
    fn long_quotes_and_long_messages_are_cut() {
        let text = (1..=9).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let msg = feedback_message("b", &[item("a", 1, 9, &text, "x")]);
        assert!(msg.starts_with("Review feedback on b (1 comment)."));
        assert!(msg.contains("   > line 6\n   > …\n   x"));
        assert!(!msg.contains("line 7"));

        let big = "y".repeat(10_000);
        let many: Vec<_> = (1..=6).map(|i| item("f", i, i, "l", &big)).collect();
        let msg = feedback_message("b", &many);
        assert!(msg.len() <= MAX_FEEDBACK_BYTES);
        assert!(msg.ends_with("…and 3 more comments, see the Changes view."), "{}", &msg[msg.len() - 80..]);
    }

    #[test]
    fn a_blank_quoted_line_has_no_trailing_space() {
        let msg = feedback_message("b", &[item("a", 1, 3, "x\n\ny", "z")]);
        assert!(msg.contains("   > x\n   >\n   > y\n"));
    }

    #[test]
    fn the_record_keeps_each_comment_with_its_place_and_lines() {
        let mut reply = item("docs/a.md", 3, 3, "x", "Still there.");
        reply.reply_to = Some("Use the API path".into());
        let mut removed = item("web/x.tsx", 40, 40, "const a = `b`;", "Why?");
        removed.side = CommentSide::Old;
        let rec =
            feedback_record(&[removed, item("crates/a.rs", 19, 20, "fn a(\n```", "Name it.\n\nAnd test it."), reply]);
        assert_eq!(
            rec,
            "**`crates/a.rs:19-20`**\n\n````\nfn a(\n```\n````\n\nName it.\n\nAnd test it.\n\n\
             **Reply on `docs/a.md:3`** (\"Use the API path\")\n\nStill there.\n\n\
             **`web/x.tsx:40`** (removed lines)\n\n```\nconst a = `b`;\n```\n\nWhy?"
        );
    }

    #[test]
    fn a_long_record_counts_what_it_leaves_out() {
        let big = "y".repeat(10_000);
        let many: Vec<_> = (1..=6).map(|i| item("f", i, i, "l", &big)).collect();
        let rec = feedback_record(&many);
        assert!(rec.len() <= MAX_FEEDBACK_BYTES);
        assert!(rec.ends_with("…and 3 more comments."), "{}", &rec[rec.len() - 40..]);
    }

    #[test]
    fn the_forge_merge_method_reads_its_old_name() {
        assert_eq!(serde_json::from_str::<MergeMethod>("\"github\"").unwrap(), MergeMethod::Forge);
        assert_eq!(serde_json::to_string(&MergeMethod::Forge).unwrap(), "\"forge\"");
    }

    #[test]
    fn the_squash_message_names_the_ticket_and_branch() {
        assert_eq!(
            squash_message("PDT-3", "Review", "pdt/PDT-3-review"),
            "PDT-3: Review\n\nSquashed from pdt/PDT-3-review."
        );
    }

    #[test]
    fn the_open_pr_message_names_the_branches() {
        assert_eq!(
            open_pr_message("f", "main"),
            "f is ready to merge, and this project merges through pull requests. Commit anything left, push f and \
             open a pull request against main (`change_request_open`)."
        );
    }

    #[test]
    fn the_conflict_message_names_the_files() {
        assert_eq!(
            conflict_message("f", "main", &["a.txt".into(), "b.rs".into()]),
            "Merging f into main conflicts in these files:\n- a.txt\n- b.rs\n\nMerge main into f, resolve the \
             conflicts, run the checks and commit."
        );
    }
}
