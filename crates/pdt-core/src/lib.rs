//! Pure domain types for Personal Dev Team. No I/O lives in this crate.

pub mod activity;
pub mod agent_context;
pub mod brand;
pub mod bug_report;
pub mod conversation;
pub mod fence;
pub mod forge;
pub mod git_url;
pub mod github_issue;
pub mod handoff;
pub mod host;
pub mod host_path;
pub mod identity;
pub mod inbox;
pub mod instance;
pub mod integration;
pub mod links;
pub mod liveness;
pub mod local_network;
pub mod mcp;
pub mod notification;
pub mod overlap;
pub mod parts;
pub mod persona;
pub mod pm;
pub mod policy;
pub mod pr;
pub mod project;
pub mod rank;
pub mod release;
pub mod review;
pub mod run;
pub mod runconfig;
pub mod secret;
pub mod session;
pub mod start;
pub mod ticket;
pub mod todo;
pub mod workflow;
pub mod workspace;
pub mod worktree;

/// The version this build reports: `PDT_BUILD_VERSION` at compile time (release builds set it to the checked-in
/// version, e.g. `0.2.0`), otherwise `<crate version>-local`, which never counts as newer than a release.
pub const BUILD_VERSION: &str = match option_env!("PDT_BUILD_VERSION") {
    Some(v) => v,
    None => concat!(env!("CARGO_PKG_VERSION"), "-local"),
};
