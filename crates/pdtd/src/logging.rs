use anyhow::Context;
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// Logs to `<log_dir>/pdtd.<date>.log` (daily, 14 files kept) and, with `to_stderr`, to stderr too. Under pdt-host
/// stderr goes to `pdtd-launch.log`, which would otherwise be an unrotated copy of the whole log. Keep the guard alive.
pub fn init(log_dir: &Path, to_stderr: bool) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir).with_context(|| format!("creating log dir {}", log_dir.display()))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("pdtd")
        .filename_suffix("log")
        .max_log_files(14)
        .build(log_dir)
        .context("creating the log file")?;
    let (file, guard) = tracing_appender::non_blocking(appender);
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(to_stderr.then(|| fmt::layer().with_writer(std::io::stderr).with_filter(filter())))
        .with(fmt::layer().with_ansi(false).with_writer(file).with_filter(filter()))
        .try_init()
        .context("initialising logging")?;
    log_panics();
    Ok(guard)
}

/// Sends every panic's message and place to the log too (stderr alone goes to `pdtd-launch.log` under pdt-host, or
/// nowhere), then does what Rust does by default.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        tracing::error!(panic = %msg, at = %at, thread = %thread, "panicked");
        default(info);
    }));
}
