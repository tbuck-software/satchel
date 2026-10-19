//! Sync paths that need the Tauri handle: the queue worker and the IDLE
//! supervisor emit window events, maintenance resolves app-data paths, and the
//! outbox reaches into the PGP/S-MIME unlock sessions. Everything else lives in
//! `satchel_core::sync` and is re-exported here so `crate::sync::…` keeps
//! resolving.
pub use satchel_core::sync::*;

pub mod attachment_processing;
pub mod delivery_reconciliation;
pub mod gmail_reconcile;
pub mod idle;
pub mod imap_workflow_target;
pub mod leases;
pub mod mailbox_sync;
pub mod maintenance;
pub mod message_ingest;
pub mod outbox;
pub mod outlook_migration;
pub mod queue_worker;
pub mod repairs;
pub mod workflow_executor;
