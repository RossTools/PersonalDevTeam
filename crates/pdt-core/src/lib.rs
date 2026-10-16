//! Pure domain types for Personal Dev Team. No I/O lives in this crate.

pub mod brand;
pub mod host;
pub mod identity;
pub mod project;
pub mod session;
pub mod worktree;

/// The version this build reports: `PDT_BUILD_VERSION` at compile time (CI release builds set it to the release
/// version, e.g. `0.1.42`), otherwise the crate version.
pub const BUILD_VERSION: &str = match option_env!("PDT_BUILD_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
