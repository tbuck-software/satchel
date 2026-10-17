//! Build script. Beyond the usual `tauri_build`, this declares the app's own
//! commands so Tauri writes an app ACL manifest.
//!
//! Without one, Tauri checks the ACL for plugin commands only — see
//! `tauri::webview`: `if (plugin_command.is_some() || has_app_acl_manifest)`.
//! Every `#[tauri::command]` was therefore reachable from every window,
//! including the one that renders a stranger's `.eml`. The capability files
//! claimed otherwise.
//!
//! The list has to match `generate_handler!` in `src/lib.rs`; a command missing
//! here has no permission to grant and cannot be called from anywhere.
//! `tests/app_acl.rs` compares the two.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Every command registered with `generate_handler!`.
const COMMANDS: &[&str] = &[
    "check_app_update",
    "download_app_update",
    "app_update_progress",
    "install_app_update",
    "acknowledge_app_update_flush",
    "list_accounts",
    "add_account",
    "deactivate_account",
    "delete_account",
    "test_imap_account_connection",
    "update_account_password",
    "clear_account_oauth_error",
    "list_mailboxes",
    "create_folder",
    "rename_folder",
    "delete_folder",
    "list_messages_preview",
    "transition_threads",
    "take_pending_open_with",
    "acknowledge_draft_flush",
    "abort_oauth_flow",
    "list_open_oauth_flows",
    "list_message_bodies",
    "load_thread_messages",
    "get_message_source",
    "search_message_hits",
    "server_search",
    "fetch_server_search_hit",
    "send_message",
    "list_contacts",
    "search_contacts",
    "sync_contact_avatars",
    "sync_local_contact_avatars",
    "upsert_contact",
    "set_contact_avatar",
    "delete_contact",
    "record_frontend_diagnostic",
    "open_external_url",
    "show_in_folder",
    "list_cached_threads",
    "list_threads",
    "list_smart_inbox_summary",
    "get_thread",
    "get_threads",
    "message_link_target",
    "resolve_message_link",
    "update_thread_state",
    "update_thread_organization",
    "set_thread_tags",
    "trash_threads",
    "set_threads_read",
    "list_recent_transitions",
    "run_imap_sync",
    "test_imap_connection",
    "discover_account",
    "detect_mail_provider",
    "autoconfigure_groupware",
    "start_oauth_flow",
    "poll_oauth_flow",
    "complete_oauth_flow",
    "oauth_status",
    "list_pgp_keys",
    "pgp_engine_status",
    "import_pgp_keys",
    "validate_pgp_bundle_passphrase",
    "generate_pgp_key",
    "unlock_pgp_keys",
    "delete_pgp_key",
    "list_smime_certificates",
    "add_smime_certificate",
    "import_smime_pkcs12",
    "generate_smime_keypair",
    "unlock_smime_certificates",
    "delete_smime_certificate",
    "list_rules",
    "upsert_rule",
    "delete_rule",
    "run_rules_for_thread",
    "run_rules_for_threads",
    "get_source_capabilities",
    "list_sources",
    "create_source",
    "delete_source",
    "poll_source",
    "import_opml",
    "list_plugins",
    "set_plugin_permission",
    "set_plugin_enabled",
    "execute_plugin_command",
    "list_plugin_audit_log",
    "list_plugin_metrics",
    "save_draft",
    "list_drafts",
    "load_draft",
    "delete_draft",
    "list_signatures",
    "upsert_signature",
    "delete_signature",
    "list_signature_assignments",
    "upsert_signature_assignment",
    "resolve_signature",
    "discover_signature_suggestions",
    "list_signature_suggestions",
    "set_signature_suggestion_status",
    "list_aliases",
    "upsert_alias",
    "delete_alias",
    "list_templates",
    "upsert_template",
    "delete_template",
    "render_template",
    "list_follow_ups",
    "upsert_follow_up",
    "cancel_follow_up",
    "set_thread_category",
    "set_thread_priority",
    "set_sender_preference",
    "get_sender_preference",
    "respond_to_invite",
    "list_invites",
    "list_calendar_events",
    "list_calendar_sources",
    "create_calendar_event",
    "delete_calendar_event",
    "sync_google_calendar",
    "import_ics",
    "import_ics_path",
    "configure_groupware",
    "sync_groupware",
    "list_outbox",
    "retry_outbox_action",
    "retry_failed_sync_actions",
    "cancel_outbox_action",
    "undo_outbox_send",
    "list_failed_sync_actions",
    "discard_failed_sync_action",
    "set_quit_on_close",
    "set_autostart",
    "set_start_minimized",
    "choose_save_path",
    "choose_directory",
    "tray_preferences",
    "tray_snapshot",
    "open_tray_thread",
    "open_tray_destination",
    "act_on_tray_thread",
    "set_thread_muted",
    "mark_threads_spam",
    "set_threads_starred",
    "list_blocked_senders",
    "block_sender",
    "unblock_sender",
    "list_smart_folders",
    "upsert_smart_folder",
    "delete_smart_folder",
    "list_topics",
    "upsert_topic",
    "delete_topic",
    "add_topic_items",
    "remove_topic_item",
    "list_topic_memberships",
    "get_topic_timeline",
    "upsert_topic_note",
    "delete_topic_note",
    "add_topic_file",
    "list_topic_files",
    "download_topic_file",
    "delete_topic_file",
    "export_mailbox",
    "import_mbox",
    "export_platform_data",
    "import_platform_data",
    "cancel_import",
    "create_backup",
    "list_backups",
    "restore_backup",
    "cancel_pending_restore",
    "prune_backups",
    "credentials_status",
    "list_provider_labels",
    "refresh_provider_labels",
    "create_provider_label",
    "get_vacation",
    "set_vacation",
    "list_webhooks",
    "upsert_webhook",
    "delete_webhook",
    "unsubscribe",
    "list_attachments_for_messages",
    "download_attachment",
    "open_attachment_default",
    "toggle_attachment_quick_look",
    "inline_image_cache_status",
    "start_inline_image_backfill",
    "stop_inline_image_backfill",
    "move_threads_to_folder",
    "set_notification_preferences",
    "get_notification_preferences",
    "open_compose_window",
    "claim_draft_editor",
    "release_draft_editor",
    "focus_draft_editor",
    "set_ui_strings",
    "current_connection_state",
    "diagnostics_overview",
    "diagnostics_report",
    "app_build_info",
    "open_logs_folder",
    "pending_crash_reports",
    "open_message_file",
    "read_message_file_attachment",
    "open_message_file_in_window",
    "runtime_capabilities",
    "set_background_sync",
    "background_sync_status",
    "open_background_sync_settings",
    "load_message_body",
    "take_pending_notification_thread",
    "consume_pending_mobile_notification",
    "dismiss_mobile_notifications",
    "mobile_notifications_frontend_ready",
];

/// Permission set covering all of [`COMMANDS`].
///
/// The main window, the composer popouts and the mobile shell all run the full
/// client bundle, so narrowing them would be fiction. Generated rather than
/// kept by hand: a set that drifts from the command list is how a command
/// becomes silently unreachable.
fn write_full_command_set() {
    let permissions = COMMANDS
        .iter()
        .map(|command| format!("  \"allow-{}\"", command.replace('_', "-")))
        .collect::<Vec<_>>()
        .join(",\n");
    let contents = format!(
        "# Automatically generated by build.rs - DO NOT EDIT!\n\
         [[set]]\n\
         identifier = \"allow-app-commands\"\n\
         description = \"Every Satchel command. For windows that run the full client.\"\n\
         permissions = [\n{permissions}\n]\n"
    );
    let path = Path::new("permissions/allow-app-commands.toml");
    fs::create_dir_all("permissions").expect("create permissions directory");
    if fs::read_to_string(path).ok().as_deref() != Some(contents.as_str()) {
        fs::write(path, contents).expect("write the full command set");
    }
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

/// Embeds the commit as `SATCHEL_GIT_SHA` for crash entries, the log and the
/// Diagnostics pane. A build without git (source archive) may set the variable.
fn emit_git_revision() {
    println!("cargo:rerun-if-env-changed=SATCHEL_GIT_SHA");
    let commit = std::env::var("SATCHEL_GIT_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git(&["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SATCHEL_GIT_SHA={commit}");

    // Only paths that exist: Cargo treats a missing one as always stale.
    let mut watched = vec!["HEAD".to_string(), "packed-refs".to_string()];
    if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watched.push(head_ref);
    }
    for entry in watched {
        if let Some(path) = git(&["rev-parse", "--git-path", &entry]) {
            if Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=SATCHEL_RELEASE_CHANNEL");
    if let Ok(channel) = std::env::var("SATCHEL_RELEASE_CHANNEL") {
        assert!(
            matches!(channel.as_str(), "stable" | "nightly"),
            "invalid release channel"
        );
        println!("cargo:rustc-env=SATCHEL_RELEASE_CHANNEL={channel}");
    }
    emit_git_revision();
    write_full_command_set();
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build");
}
