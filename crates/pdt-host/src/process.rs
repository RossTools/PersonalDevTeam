//! Starting child processes without a console window on Windows (PDT-60). pdt-host has no console of its own (release
//! builds use the windows subsystem), so Windows gives every console program it runs (powershell, taskkill, reg, the
//! installer's helpers) a new, visible console window unless asked not to.

use std::process::Command;

/// `CREATE_NO_WINDOW`: a console program runs with a hidden console. No effect on GUI programs.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `CREATE_NEW_PROCESS_GROUP`: the child does not get this process group's Ctrl+C.
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Runs `cmd` without a console window (Windows); unchanged elsewhere.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs `cmd` without a console window and in its own process group (Windows); unchanged elsewhere. For processes
/// that outlive this one (pdtd, a replacement pdt-host).
pub fn detached_no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
    cmd
}
