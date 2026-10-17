// Generated from Rust. Run pnpm ipc:generate.
export type SyncSummaryDto = {
    accounts_synced: number;
    accounts_failed: number;
    folders_synced: number;
    messages_synced: number;
    backfill_pending_accounts: number;
    backfill_remaining_messages: number;
    backfill_total_messages: number;
    failures: Array<string>;
};
