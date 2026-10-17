// Generated from Rust. Run pnpm ipc:generate.
export type DiagnosticsAccountDto = {
    id: string;
    email: string;
    display_name: string;
    provider: string;
    sync_status: string;
    last_sync_at: string | null;
    last_sync_error: string | null;
    last_sync_error_at: string | null;
    last_oauth_error: string | null;
    last_oauth_error_at: string | null;
};
