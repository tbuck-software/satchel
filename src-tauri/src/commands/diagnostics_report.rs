//! The "Copy diagnostics" text and the "Open logs folder" action.
//!
//! The report is plain text for a bug report: build, platform, the
//! Diagnostics snapshot without names or content, and the latest warnings and
//! errors from the log. Every line passes through [`redact`] before it leaves.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::commands::diagnostics::{build_overview, DiagnosticsOverviewDto};
use crate::commands::error::{CommandError, CommandResult};
use crate::utils::build_info::{self, BuildInfo};
use crate::utils::redact::redact;
use crate::utils::telemetry::{self, LOG_FILE_PREFIX};

/// Enough to see what led up to a failure without pasting a day of log.
const MAX_LOG_ENTRIES: usize = 50;
const MAX_ENTRY_CHARS: usize = 2_000;
/// Read from the end of each of the two newest log files.
const LOG_TAIL_BYTES: u64 = 512 * 1024;

static ENTRY_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\d{4}-\d{2}-\d{2}T\S+\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s")
        .expect("static log entry pattern")
});

#[tauri::command]
pub fn app_build_info() -> BuildInfo {
    build_info::current()
}

#[tauri::command]
pub async fn diagnostics_report(
    pool: tauri::State<'_, crate::db::ReadPool>,
) -> CommandResult<String> {
    let overview = build_overview(pool.inner().as_ref()).await?;
    let log = telemetry::log_dir()
        .map(recent_log_problems)
        .unwrap_or_default();
    Ok(render_report(&build_info::current(), &overview, &log))
}

#[tauri::command]
pub fn open_logs_folder(app: tauri::AppHandle) -> CommandResult<()> {
    use tauri_plugin_opener::OpenerExt;

    let unavailable = || CommandError::new("backend.errors.logsFolderUnavailable");
    let directory = telemetry::log_dir().ok_or_else(unavailable)?;
    app.opener()
        .open_path(directory.to_string_lossy(), None::<&str>)
        .map_err(|err| {
            tracing::warn!(target: "telemetry", "open logs folder: {err}");
            unavailable()
        })
}

/// Account ids become `account 1`, `account 2`, … in the snapshot's order, so
/// rows stay correlatable without the address.
fn account_labels(overview: &DiagnosticsOverviewDto) -> HashMap<&str, String> {
    overview
        .accounts
        .iter()
        .enumerate()
        .map(|(index, account)| (account.id.as_str(), format!("account {}", index + 1)))
        .collect()
}

fn render_report(build: &BuildInfo, overview: &DiagnosticsOverviewDto, log: &[String]) -> String {
    let labels = account_labels(overview);
    let account = |id: &str| {
        labels
            .get(id)
            .cloned()
            .unwrap_or_else(|| "other account".into())
    };
    let or_dash = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".into());
    let mut out = String::new();

    let _ = writeln!(out, "Satchel diagnostics (redacted)");
    let _ = writeln!(out, "version: {}", build.version);
    let _ = writeln!(out, "commit: {}", build.commit);
    let _ = writeln!(out, "platform: {}/{}", build.os, build.arch);
    let _ = writeln!(
        out,
        "profile: {}",
        if build.debug { "debug" } else { "release" }
    );
    let _ = writeln!(out, "generated: {}", overview.generated_at);

    let _ = writeln!(out, "\n## Accounts");
    for entry in &overview.accounts {
        let _ = writeln!(
            out,
            "- {}: provider={} status={} last_sync={}",
            account(&entry.id),
            entry.provider,
            entry.sync_status,
            or_dash(&entry.last_sync_at),
        );
        if let Some(error) = &entry.last_sync_error {
            let _ = writeln!(
                out,
                "  sync_error ({}): {error}",
                or_dash(&entry.last_sync_error_at)
            );
        }
        if let Some(error) = &entry.last_oauth_error {
            let _ = writeln!(
                out,
                "  oauth_error ({}): {error}",
                or_dash(&entry.last_oauth_error_at)
            );
        }
    }

    let _ = writeln!(out, "\n## Sync queue");
    for bucket in &overview.queue.by_status {
        let _ = writeln!(
            out,
            "- {}: {} entries, {} retries (max {})",
            bucket.status, bucket.entries, bucket.total_retries, bucket.max_retries
        );
    }
    for entry in &overview.queue.recent_failures {
        let _ = writeln!(
            out,
            "- failure {} ({}): status={} attempts={} updated={} error={}",
            entry.action_type,
            account(&entry.account_id),
            entry.status,
            entry.attempts,
            entry.updated_at,
            or_dash(&entry.error),
        );
    }

    let _ = writeln!(out, "\n## Notification actions");
    for bucket in &overview.notifications.by_status {
        let _ = writeln!(
            out,
            "- {}: {} entries, {} retries",
            bucket.status, bucket.entries, bucket.total_retries
        );
    }

    // Subjects and senders stay out; states and reason codes say enough.
    let outbox = &overview.notification_outbox;
    let _ = writeln!(out, "\n## Notification outbox");
    for bucket in &outbox.by_state {
        let _ = writeln!(out, "- {}: {}", bucket.state, bucket.entries);
    }
    for bucket in &outbox.reasons_24h {
        let _ = writeln!(
            out,
            "- suppressed (24 h) {}: {}",
            bucket.reason, bucket.entries
        );
    }

    let _ = writeln!(out, "\n## Sync leases");
    for lease in &overview.sync_leases {
        let state = if lease.expired { "expired" } else { "active" };
        let _ = writeln!(
            out,
            "- {}: {state} expires={} renewed={}",
            account(&lease.account_id),
            lease.lease_expires_at,
            lease.updated_at
        );
    }

    let _ = writeln!(out, "\n## Rate limits");
    for limit in &overview.rate_limits {
        let _ = writeln!(
            out,
            "- {}: blocked_until={} streak={}",
            limit.provider,
            or_dash(&limit.blocked_until),
            limit.streak
        );
    }

    let _ = writeln!(out, "\n## Recent warnings and errors");
    if log.is_empty() {
        let _ = writeln!(out, "(none)");
    }
    for entry in log {
        let _ = writeln!(out, "{entry}");
    }

    redact(&out)
}

/// The last WARN and ERROR entries of the two newest daily files, oldest
/// first. Continuation lines (a crash report's backtrace) stay with their entry.
fn recent_log_problems(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(LOG_FILE_PREFIX))
        })
        .collect::<Vec<_>>();
    // `satchel.log.YYYY-MM-DD` sorts by date.
    files.sort();
    let newest = &files[files.len().saturating_sub(2)..];

    let mut problems = Vec::new();
    for path in newest {
        let Some(text) = read_tail(path, LOG_TAIL_BYTES) else {
            continue;
        };
        let mut current: Option<String> = None;
        for line in text.lines() {
            if ENTRY_START.is_match(line) {
                keep_if_problem(current.take(), &mut problems);
                current = Some(line.to_string());
            } else if let Some(entry) = current.as_mut() {
                entry.push('\n');
                entry.push_str(line);
            }
        }
        keep_if_problem(current, &mut problems);
    }
    let skip = problems.len().saturating_sub(MAX_LOG_ENTRIES);
    problems.split_off(skip)
}

fn keep_if_problem(entry: Option<String>, problems: &mut Vec<String>) {
    let Some(entry) = entry else { return };
    let level = ENTRY_START
        .captures(&entry)
        .and_then(|caps| caps.get(1))
        .map(|level| level.as_str());
    if matches!(level, Some("WARN" | "ERROR")) {
        problems.push(truncate(entry, MAX_ENTRY_CHARS));
    }
}

fn read_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Some(text);
    }
    // The first line was cut mid-way.
    Some(
        text.split_once('\n')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or_default(),
    )
}

fn truncate(mut entry: String, max_chars: usize) -> String {
    if let Some((index, _)) = entry.char_indices().nth(max_chars) {
        entry.truncate(index);
        entry.push_str(" …");
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnostics::{
        DiagnosticsAccountDto, DiagnosticsNotificationOutboxDto, DiagnosticsNotificationsDto,
        DiagnosticsOutboxEntryDto, DiagnosticsQueueDto, DiagnosticsQueueEntryDto,
        DiagnosticsSyncLeaseDto,
    };

    fn overview() -> DiagnosticsOverviewDto {
        DiagnosticsOverviewDto {
            generated_at: "2026-10-13T08:00:00Z".into(),
            accounts: vec![DiagnosticsAccountDto {
                id: "acc-1".into(),
                email: "jane@firma-intern.de".into(),
                display_name: "Jane Doe".into(),
                provider: "imap".into(),
                sync_status: "error".into(),
                last_sync_at: Some("2026-10-13T07:00:00Z".into()),
                last_sync_error: Some(
                    "{\"key\":\"backend.errors.accountSyncFailed\",\"params\":{\"account\":\"jane@firma-intern.de\",\"message\":\"connect imap.firma-intern.de:993 failed\"}}".into(),
                ),
                last_sync_error_at: Some("2026-10-13T07:30:00Z".into()),
                last_oauth_error: None,
                last_oauth_error_at: None,
            }],
            queue: DiagnosticsQueueDto {
                by_status: Vec::new(),
                recent_failures: vec![DiagnosticsQueueEntryDto {
                    id: "q-1".into(),
                    account_id: "acc-1".into(),
                    action_type: "send_message".into(),
                    status: "failed".into(),
                    error: Some("SMTP 550 rejected for bob@kunde.example-corp.com".into()),
                    attempts: 5,
                    next_due: None,
                    updated_at: "2026-10-13T07:45:00Z".into(),
                }],
            },
            notifications: DiagnosticsNotificationsDto {
                by_status: Vec::new(),
                recent: Vec::new(),
            },
            notification_outbox: DiagnosticsNotificationOutboxDto {
                by_state: Vec::new(),
                reasons_24h: Vec::new(),
                recent: vec![DiagnosticsOutboxEntryDto {
                    message_id: "m-1".into(),
                    state: "dismissed".into(),
                    created_at: "2026-10-13T07:00:00Z".into(),
                    delivered_at: None,
                    reason: Some("filter_already_read".into()),
                    detail: None,
                    subject: Some("Kündigung Mietvertrag".into()),
                    from_name: Some("Vermieter Müller".into()),
                    from_address: Some("mueller@vermieter.de".into()),
                    date: None,
                    account_id: Some("acc-1".into()),
                }],
            },
            sync_leases: vec![DiagnosticsSyncLeaseDto {
                account_id: "acc-1".into(),
                email: Some("jane@firma-intern.de".into()),
                owner_token: "worker-1".into(),
                lease_expires_at: "2026-10-13T08:05:00Z".into(),
                updated_at: "2026-10-13T08:00:00Z".into(),
                expired: false,
            }],
            rate_limits: Vec::new(),
        }
    }

    #[test]
    fn report_names_the_build_and_leaves_people_and_content_out() {
        let log = vec![
            "2026-10-13T07:30:00Z ERROR satchel::sync: login failed for jane@firma-intern.de password=hunter2".to_string(),
        ];

        let report = render_report(&build_info::current(), &overview(), &log);

        assert!(
            report.contains(&format!("version: {}", build_info::VERSION)),
            "{report}"
        );
        assert!(
            report.contains(&format!("commit: {}", build_info::COMMIT)),
            "{report}"
        );
        assert!(
            report.contains("account 1: provider=imap status=error"),
            "{report}"
        );
        assert!(
            report.contains("backend.errors.accountSyncFailed"),
            "{report}"
        );
        assert!(
            report.contains("login failed for <email> password=<redacted>"),
            "{report}"
        );
        for leaked in [
            "jane",
            "Jane Doe",
            "firma-intern",
            "bob@",
            "example-corp",
            "Kündigung",
            "Müller",
            "vermieter",
            "hunter2",
        ] {
            assert!(!report.contains(leaked), "{leaked} leaked:\n{report}");
        }
    }

    #[test]
    fn log_excerpt_keeps_warnings_and_errors_with_their_continuation_lines() {
        let directory = tempfile::tempdir().expect("log directory");
        std::fs::write(
            directory.path().join("satchel.log.2026-10-11"),
            "2026-10-11T08:00:00Z ERROR old: too old to matter\n",
        )
        .expect("oldest log");
        std::fs::write(
            directory.path().join("satchel.log.2026-10-12"),
            "2026-10-12T08:00:00Z  WARN satchel::sync: yesterday\n",
        )
        .expect("older log");
        std::fs::write(
            directory.path().join("satchel.log.2026-10-13"),
            "2026-10-13T08:00:00Z  INFO satchel::frontend: ui.click\n\
             2026-10-13T08:00:01Z ERROR satchel: crash from previous run report===== panic\n\
             backtrace line\n\
             2026-10-13T08:00:02Z DEBUG satchel: noise\n",
        )
        .expect("newest log");
        std::fs::write(directory.path().join("crash.log"), "ignored").expect("crash log");

        let problems = recent_log_problems(directory.path());

        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].ends_with("yesterday"));
        assert!(
            problems[1].ends_with("panic\nbacktrace line"),
            "{problems:?}"
        );
    }

    #[test]
    fn log_excerpt_is_bounded() {
        let directory = tempfile::tempdir().expect("log directory");
        let mut text = String::new();
        for index in 0..(MAX_LOG_ENTRIES + 10) {
            let _ = writeln!(
                text,
                "2026-10-13T08:00:00Z  WARN satchel: entry {index} {}",
                "x".repeat(3_000)
            );
        }
        std::fs::write(directory.path().join("satchel.log.2026-10-13"), text).expect("log");

        let problems = recent_log_problems(directory.path());

        assert_eq!(problems.len(), MAX_LOG_ENTRIES);
        assert!(problems[0].contains("entry 10 "));
        assert!(problems
            .iter()
            .all(|entry| entry.chars().count() <= MAX_ENTRY_CHARS + 2));
    }
}
