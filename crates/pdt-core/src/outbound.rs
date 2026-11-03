//! The limits PDT's outbound HTTP clients share (the forges, workflows' nodes, toolchain installs, updates); each
//! names itself with [`crate::brand::user_agent`].

use std::time::Duration;

/// The longest connecting to a server may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest a download (an update, a toolchain's archive) may take, end to end.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
