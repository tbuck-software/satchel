//! Satchel's domain: database, sync queue, mail providers, MIME and crypto.
//!
//! Nothing here links against Tauri. The desktop app wires these modules to
//! windows, commands and platform plugins; the hub (ADR 0008) will reuse them
//! headless.
//
// Two clippy style lints are allowed crate-wide rather than sprinkled as local
// attributes, because both fire on patterns that are idiomatic here and not
// defects:
//
// - `type_complexity`: sqlx `query_as` returns anonymous tuples whose shape is
//   the query's column list. Naming each one adds indirection without adding
//   information.
// - `too_many_arguments`: the message-mapping helpers thread request-scoped
//   context (pool, key availability, provider config) explicitly instead of
//   hiding it in shared mutable state. That is the safer trade.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

pub mod accounts;
pub mod backup;
pub mod calendar;
pub mod db;
pub mod drafts;
pub mod email;
pub mod events;
pub mod mail_actions;
pub mod providers;
pub mod runtime_capabilities;
pub mod sync;
pub mod utils;
pub mod workflow;
