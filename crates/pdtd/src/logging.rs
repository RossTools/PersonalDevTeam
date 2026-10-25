use anyhow::Context;
use pdt_core::secret::Redactor;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// A log writer that redacts known secrets' values. The fmt layer writes each event in one go, so a value is never
/// split between writes.
struct Redacting<M> {
    inner: M,
    redactor: Arc<Redactor>,
}

struct RedactingWriter<W> {
    inner: W,
    redactor: Arc<Redactor>,
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for Redacting<M> {
    type Writer = RedactingWriter<M::Writer>;
    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter { inner: self.inner.make_writer(), redactor: self.redactor.clone() }
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.redactor.is_empty() {
            return self.inner.write(buf);
        }
        let text = String::from_utf8_lossy(buf);
        self.inner.write_all(self.redactor.text(&text).as_bytes())?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Logs to `<log_dir>/pdtd.<date>.log` (daily, 14 files kept) and, with `to_stderr`, to stderr too. Under pdt-host
/// stderr goes to `pdtd-launch.log`, which would otherwise be an unrotated copy of the whole log. Keep the guard alive.
/// Known secrets' values are replaced in every line (`redactor`, PDT-158).
pub fn init(log_dir: &Path, to_stderr: bool, redactor: Arc<Redactor>) -> anyhow::Result<WorkerGuard> {
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
    let stderr = Redacting { inner: std::io::stderr, redactor: redactor.clone() };
    let file = Redacting { inner: file, redactor };
    tracing_subscriber::registry()
        .with(to_stderr.then(|| fmt::layer().with_writer(stderr).with_filter(filter())))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_are_redacted() {
        let r = Arc::new(Redactor::default());
        let mut w = RedactingWriter { inner: Vec::new(), redactor: r.clone() };
        w.write_all(b"plain line\n").unwrap();
        r.set("TOKEN", "hunter2-value");
        w.write_all(b"token=hunter2-value end\n").unwrap();
        assert_eq!(String::from_utf8(w.inner).unwrap(), "plain line\ntoken=[secret:TOKEN] end\n");
    }
}
