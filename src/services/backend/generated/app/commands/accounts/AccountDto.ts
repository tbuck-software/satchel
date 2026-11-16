// Generated from Rust. Run pnpm ipc:generate.
export type AccountDto = {
    id: string;
    /**
     * The name on outgoing mail.
     */
    display_name: string;
    /**
     * The account's name in Satchel's lists; `null` shows `display_name`.
     */
    label: string | null;
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
    /**
     * A `data:` URL the user chose for the account, at most 128px.
     */
    avatar: string | null;
};
