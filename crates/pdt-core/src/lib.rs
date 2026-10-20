//! Pure domain types for Personal Dev Team. No I/O lives in this crate.

pub mod activity;
pub mod agent_context;
pub mod brand;
pub mod conversation;
pub mod git_url;
pub mod handoff;
pub mod host;
pub mod host_path;
pub mod identity;
pub mod instance;
pub mod links;
pub mod overlap;
pub mod parts;
pub mod pr;
pub mod project;
pub mod rank;
pub mod release;
pub mod review;
pub mod run;
pub mod runconfig;
pub mod session;
pub mod start;
pub mod ticket;
pub mod workspace;
pub mod worktree;

/// The version this build reports: `PDT_BUILD_VERSION` at compile time (CI release builds set it to the release
/// version, e.g. `0.1.42`), otherwise `<crate version>-local`, which never counts as newer than a release.
pub const BUILD_VERSION: &str = match option_env!("PDT_BUILD_VERSION") {
    Some(v) => v,
    None => concat!(env!("CARGO_PKG_VERSION"), "-local"),
};
