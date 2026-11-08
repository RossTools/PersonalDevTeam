//! The crate's integration tests in one binary, each `tests/*.rs` file a module of it (Cargo.toml sets
//! `autotests = false`): a binary per file linked all of PDT and its dependencies again for each one.

#[path = "../../../test-support/all_listed.rs"]
mod all_listed;
#[path = "../../../test-support/port_blocks.rs"]
mod port_blocks;
#[path = "../../../test-support/wait.rs"]
mod wait;

mod codex_screen;
mod pty;
