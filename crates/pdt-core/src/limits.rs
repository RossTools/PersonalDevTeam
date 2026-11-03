//! Size limits several parts share, so one change reaches them all.

/// The longest title (characters): a ticket's, a document's, a PM note's, a bug report's.
pub const TITLE: usize = 200;
/// The longest comment (characters): on a ticket or in a document.
pub const COMMENT: usize = 20_000;
/// How much of a command's stdout and of its stderr is kept (bytes): a workflow's command step, `run_with_secrets`,
/// a conflict step's regenerate command.
pub const OUTPUT_CAP: usize = 64 * 1024;
