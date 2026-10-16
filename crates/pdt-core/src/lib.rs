//! Pure domain types for Personal Dev Team. No I/O lives in this crate.

pub mod activity;
pub mod brand;
pub mod handoff;
pub mod host;
pub mod identity;
pub mod project;
pub mod release;
pub mod run;
pub mod runconfig;
pub mod session;
pub mod worktree;

/// The version this build reports: `PDT_BUILD_VERSION` at compile time (CI release builds set it to the release
/// version, e.g. `0.1.42`), otherwise `<crate version>-local`, which never counts as newer than a release.
pub const BUILD_VERSION: &str = match option_env!("PDT_BUILD_VERSION") {
    Some(v) => v,
    None => concat!(env!("CARGO_PKG_VERSION"), "-local"),
};
