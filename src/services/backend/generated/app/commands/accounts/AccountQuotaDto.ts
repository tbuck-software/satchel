// Generated from Rust. Run pnpm ipc:generate.
/**
 * The last quota an account's server reported. Storage is in bytes; a pair
 * is `None` when the server sets no such limit. Only accounts with at least
 * one limit are listed.
 */
export type AccountQuotaDto = {
    account_id: string;
    storage_used_bytes: number | null;
    storage_limit_bytes: number | null;
    messages_used: number | null;
    messages_limit: number | null;
    /**
     * RFC 3339; when the numbers were read.
     */
    fetched_at: string | null;
};
