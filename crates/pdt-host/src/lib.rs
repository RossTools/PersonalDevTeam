//! pdt-host: keeps pdtd running on a host, installs updates pdtd hands it, rolls back a bad one (remote-host spec §3).
//! Deliberately small and synchronous, so it rarely needs to change itself.

pub mod config;
pub mod dbsnap;
pub mod health;
pub mod install;
pub mod linux;
pub mod log;
pub mod placement;
pub mod process;
pub mod register;
pub mod selfupdate;
pub mod supervise;
pub mod tray_login;
