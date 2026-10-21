//! Process adapters: PTY terminals and the structured agent adapters (conversations).

pub mod adapter;
pub mod detached;
pub mod hooks;
pub mod instructions;
pub mod launch;
pub mod mcp;
pub mod pty;

pub use pty::{Attachment, PtyError, PtyEvent, PtyManager, SCROLLBACK_BYTES, SpawnSpec};
