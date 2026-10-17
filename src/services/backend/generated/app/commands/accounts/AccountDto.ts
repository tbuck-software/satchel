// Generated from Rust. Run pnpm ipc:generate.
export type AccountDto = {
    id: string;
    display_name: string;
    email: string;
    provider: string;
    kind: string;
    color: string | null;
    last_oauth_error: string | null;
    last_oauth_error_at: string | null;
    last_sync_at: string | null;
    sync_status: string | null;
    last_sync_error: string | null;
    last_sync_error_at: string | null;
    sync_progress_json: string | null;
};
