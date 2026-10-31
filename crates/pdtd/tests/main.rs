//! The crate's integration tests in one binary, each `tests/*.rs` file a module of it (Cargo.toml sets
//! `autotests = false`): a binary per file linked all of PDT and its dependencies again for each one.

#[path = "../../../test-support/all_listed.rs"]
mod all_listed;
mod common;

mod hooks;
mod lifecycle;
