//! Logging + crash reporting infrastructure.
//!
//! - Routes everything through `tracing` so we can flip to JSON for ingest later.
//! - Writes daily-rotated log files to the app's log directory, keeping [`MAX_LOG_FILES`].
//! - Installs a panic handler that captures the panic + backtrace into a crash log so we
//!   can show the user a "report this" dialog after restart.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{filter::EnvFilter, fmt, prelude::*};

use crate::utils::build_info;

/// Daily files kept; a phone has no user who would ever clean them up.
const MAX_LOG_FILES: usize = 14;
/// Same names as the earlier `rolling::daily` files, so those are pruned too.
pub const LOG_FILE_PREFIX: &str = "satchel.log";

static GUARD: OnceLock<WorkerGuard> = OnceLock::new();
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();
static CRASH_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static ANDROID_EXIT_REPORT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Where the rotating log lives, once [`init`] has run.
pub fn log_dir() -> Option<&'static Path> {
    LOG_DIR.get().map(PathBuf::as_path)
}

pub fn init(log_dir: PathBuf) {
    if let Err(err) = std::fs::create_dir_all(&log_dir) {
        tracing::error!(target: "telemetry", "cannot create log dir {log_dir:?}: {err}");
        return;
    }
    let crash_log = log_dir.join("crash.log");
    let _ = LOG_DIR.set(log_dir.clone());
    let _ = CRASH_LOG_PATH.set(crash_log.clone());
    let _ = ANDROID_EXIT_REPORT_DIR.set(log_dir.join("android-exit"));

    let file_layer = match rolling_appender(&log_dir) {
        Ok(appender) => {
            let (non_blocking, guard) = tracing_appender::non_blocking(appender);
            let _ = GUARD.set(guard);
            Some(
                fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true),
            )
        }
        Err(err) => {
            eprintln!("cannot open log file in {log_dir:?}: {err}");
            None
        }
    };

    let env_filter = log_filter(std::env::var("SATCHEL_LOG").ok().as_deref());
    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(false);

    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::filter::filter_fn(below_secret_floor))
        .with(file_layer)
        .with(stderr_layer)
        .try_init();

    install_panic_handler(crash_log);
    let build = build_info::current();
    tracing::info!(
        version = build.version,
        commit = build.commit,
        os = build.os,
        arch = build.arch,
        "satchel starting"
    );
}

fn rolling_appender(
    log_dir: &Path,
) -> Result<RollingFileAppender, tracing_appender::rolling::InitError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(log_dir)
}

/// Targets that log protocol traffic, credentials included, below WARN.
///
/// async-imap traces the raw `LOGIN user password` line. Debugging with
/// `SATCHEL_LOG=trace` must not put the password into the daily log file.
const SECRET_BEARING_TARGETS: &[&str] = &["async_imap", "postroom_imap"];

/// `SATCHEL_LOG` (or the default), with WARN as the floor for
/// [`SECRET_BEARING_TARGETS`] whatever the variable asks for.
fn log_filter(requested: Option<&str>) -> EnvFilter {
    let base = requested
        .and_then(|value| EnvFilter::try_new(value).ok())
        .unwrap_or_else(|| EnvFilter::new("info,sqlx=warn,tauri=warn,wry=warn,reqwest=warn"));
    // A directive for the same target replaces the requested one.
    SECRET_BEARING_TARGETS.iter().fold(base, |filter, target| {
        filter.add_directive(
            format!("{target}=warn")
                .parse()
                .expect("static log directive"),
        )
    })
}

/// Holds the floor against a more specific request (`async_imap::client=trace`),
/// which would otherwise outrank the directive above.
fn below_secret_floor(metadata: &tracing::Metadata<'_>) -> bool {
    let target = metadata.target();
    let secret_bearing = SECRET_BEARING_TARGETS.iter().any(|root| {
        target == *root
            || target
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with("::"))
    });
    !secret_bearing || *metadata.level() <= tracing::Level::WARN
}

fn install_panic_handler(crash_log: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let payload = info
            .payload()
            .downcast_ref::<&'static str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("(unknown panic payload)");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "(unknown location)".into());
        let entry = panic_entry(
            &chrono::Utc::now().to_rfc3339(),
            &location,
            payload,
            &format!("{backtrace:?}"),
        );
        // Best-effort append; if logging itself panics we never make it back here.
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&crash_log)
            .and_then(|mut f| std::io::Write::write_all(&mut f, entry.as_bytes()));
        tracing::error!(target: "satchel::panic", "{location}: {payload}");
        default(info);
    }));
}

/// One `crash.log` record. The build line says which commit the backtrace
/// belongs to; without it a report from an older install points at code that
/// may no longer exist.
fn panic_entry(at: &str, location: &str, payload: &str, backtrace: &str) -> String {
    format!(
        "==== panic at {at} ====\nbuild: {}\nlocation: {location}\npayload: {payload}\nbacktrace:\n{backtrace}\n\n",
        build_info::summary()
    )
}

/// Read and clear any pending crash from a previous run. Used on startup to
/// surface "report this" UI to the user.
///
/// The report is copied into the rotating log before the file is cleared.
/// Without that the payload existed only in the toast: the banner said
/// "abgestürzt", the user dismissed it, and the backtrace naming the culprit
/// was gone. An intermittent crash is then unfixable by construction, which is
/// exactly what happened with a double `ndk_context::initialize_android_context`
/// on Android.
pub fn drain_pending_crashes() -> Vec<String> {
    let mut reports = CRASH_LOG_PATH
        .get()
        .map(|path| drain_panic_log(path))
        .unwrap_or_default();
    if let Some(directory) = ANDROID_EXIT_REPORT_DIR.get() {
        reports.extend(drain_report_directory(directory));
    }
    reports
}

fn drain_panic_log(path: &std::path::Path) -> Vec<String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if content.is_empty() {
        return Vec::new();
    }
    let reports = content
        .split("==== panic at ")
        .filter(|chunk| !chunk.trim().is_empty())
        .map(|chunk| format!("==== panic at {chunk}"))
        .collect::<Vec<_>>();
    for report in &reports {
        tracing::error!(report = %report.trim(), "crash from previous run");
    }
    let _ = std::fs::write(path, "");
    reports
}

fn drain_report_directory(directory: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect::<Vec<_>>();
    paths.sort();

    let mut reports = Vec::new();
    for path in paths {
        let Ok(report) = std::fs::read_to_string(&path) else {
            continue;
        };
        if report.trim().is_empty() {
            let _ = std::fs::remove_file(path);
            continue;
        }
        tracing::error!(report = %report.trim(), "Android exit report from previous run");
        reports.push(report);
        let _ = std::fs::remove_file(path);
    }
    reports
}

#[cfg(test)]
mod tests {
    use super::{
        below_secret_floor, drain_panic_log, drain_report_directory, log_filter, panic_entry,
        rolling_appender, MAX_LOG_FILES,
    };
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::prelude::*;

    /// Targets of the events that got through.
    fn logged_with(requested: &str) -> Vec<String> {
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let recorder = seen.clone();
        let subscriber = tracing_subscriber::registry()
            .with(log_filter(Some(requested)))
            .with(tracing_subscriber::filter::filter_fn(below_secret_floor))
            .with(tracing_subscriber::fmt::layer().with_writer(move || {
                let recorder = recorder.clone();
                RecordingWriter(recorder)
            }));
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target: "async_imap", "C: A1 LOGIN user secret");
            tracing::trace!(target: "async_imap::client", "C: A2 LOGIN user secret");
            tracing::debug!(target: "postroom_imap", "credentials");
            tracing::warn!(target: "async_imap", "connection reset");
            tracing::trace!(target: "satchel::sync", "tracing works");
        });
        let lines = seen.lock().unwrap().clone();
        lines
    }

    struct RecordingWriter(Arc<Mutex<Vec<String>>>);

    impl std::io::Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(buf).into_owned());
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn trace_logging_keeps_imap_credentials_out() {
        for requested in [
            "trace",
            "trace,async_imap=trace",
            "async_imap::client=trace,trace",
        ] {
            let logged = logged_with(requested).join("");

            assert!(!logged.contains("LOGIN"), "{requested}: {logged}");
            assert!(!logged.contains("credentials"), "{requested}: {logged}");
            assert!(logged.contains("connection reset"), "{requested}: {logged}");
            assert!(logged.contains("tracing works"), "{requested}: {logged}");
        }
    }

    #[test]
    fn opening_the_log_prunes_old_daily_files() {
        let directory = tempfile::tempdir().expect("create log directory");
        for day in 1..=20 {
            let name = format!("satchel.log.2026-09-{day:02}");
            std::fs::write(directory.path().join(name), "old").expect("write old log");
        }
        std::fs::write(directory.path().join("crash.log"), "keep").expect("write crash log");

        drop(rolling_appender(directory.path()).expect("open appender"));

        let logs = std::fs::read_dir(directory.path())
            .expect("list log directory")
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("satchel.log")
            })
            .count();
        assert_eq!(logs, MAX_LOG_FILES);
        assert!(directory.path().join("crash.log").exists());
    }

    #[test]
    fn crash_entries_name_the_build() {
        let directory = tempfile::tempdir().expect("create log directory");
        let path = directory.path().join("crash.log");
        let entry = panic_entry("2026-10-13T08:00:00Z", "src/lib.rs:1:1", "boom", "<frames>");
        std::fs::write(&path, entry).expect("write crash log");

        let reports = drain_panic_log(&path);

        assert_eq!(reports.len(), 1);
        let build_line = format!("build: {}", crate::utils::build_info::summary());
        assert!(reports[0].contains(&build_line), "{}", reports[0]);
        assert!(reports[0].contains(crate::utils::build_info::COMMIT));
        assert!(reports[0].contains("payload: boom"));
    }

    #[test]
    fn drains_only_published_android_exit_reports_in_timestamp_order() {
        let directory = tempfile::tempdir().expect("create report directory");
        std::fs::write(directory.path().join("200.anr.log"), "second")
            .expect("write second report");
        std::fs::write(directory.path().join("100.anr.log"), "first").expect("write first report");
        std::fs::write(directory.path().join(".300.tmp"), "incomplete")
            .expect("write temporary report");

        assert_eq!(
            drain_report_directory(directory.path()),
            ["first", "second"]
        );
        assert!(!directory.path().join("100.anr.log").exists());
        assert!(!directory.path().join("200.anr.log").exists());
        assert!(directory.path().join(".300.tmp").exists());
    }
}
