pub mod accounts;
pub mod aliases;
pub mod app_update;
pub mod attachments;
pub mod background_sync;
pub mod backup;
pub mod blocked_senders;
pub mod calendar;
pub mod categories;
pub mod connection;
pub mod contacts;
pub mod crash;
pub mod diagnostics;
pub mod diagnostics_report;
pub mod draft_editors;
pub mod draft_flush;
pub mod drafts;
pub mod error;
pub mod external_links;
pub mod file_limits;
pub mod folders;
pub mod followups;
pub mod groupware;
pub mod inline_cache;
pub mod mailboxes;
pub mod message_body;
pub mod message_file;
mod message_processing;
pub mod messages;
pub mod mobile_notifications;
pub mod move_threads;
pub mod notification_navigation;
pub mod notifications;
pub mod oauth;
pub mod onboarding;
pub mod open_with;
pub mod outbox;
pub mod pgp;
pub mod plugins;
pub mod provider_features;
pub mod reveal;
pub mod rules;
pub mod runtime;
pub mod write_grants;
// Kept as a sibling file, unlike the crate's inline `mod tests`: it spans
// `server_search` and `providers::traits`, while `messages.rs` is already
// past 2900 lines.
pub mod server_search;
#[cfg(test)]
mod server_search_tests;
pub mod signatures;
pub mod smart_folders;
pub mod smime;
pub mod sources;
pub mod star;
pub mod sync;
pub mod templates;
pub mod thread_actions;
pub mod threads;
pub mod topics;
#[cfg(desktop)]
pub mod tray;
#[cfg(mobile)]
#[path = "tray_mobile.rs"]
pub mod tray;
pub mod ui_strings;
pub mod unsubscribe;
pub mod vacation;
pub mod webhooks;
pub mod workflow;

// Export mobile fallback DTOs during desktop contract checks too.
#[cfg(any(mobile, all(test, feature = "ipc-types")))]
pub mod tray_mobile_dto;
