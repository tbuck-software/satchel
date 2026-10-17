pub mod attachment_fetch;
pub mod calendar_backfill;
pub mod errors;
pub mod queue;
pub mod rate_limit_state;
pub mod remote_mutations;
pub mod threading;
pub mod watcher;
pub mod worker;

/// Whether an account is the seeded local demo account rather than a real one.
///
/// Callers use this to keep the demo account out of real sync runs and IDLE
/// connections. The legacy domain has to stay recognised: profiles seeded before
/// the rename carry `@flowmail.dev`, and dropping it would silently promote the
/// demo account to a real one that the sync loop then tries to connect.
pub fn is_seeded_demo_account(id: &str, email: &str) -> bool {
    id == "acc-work" && (email.ends_with("@satchel.dev") || email.ends_with("@flowmail.dev"))
}
