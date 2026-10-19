//! Process adapters. Phase 1: PTY terminals. Phase 3 adds the structured Claude and Codex adapters.

pub mod detached;
pub mod hooks;
pub mod launch;
pub mod pty;

pub use pty::{Attachment, PtyError, PtyEvent, PtyManager, SCROLLBACK_BYTES, SpawnSpec};
