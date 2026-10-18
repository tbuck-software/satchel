// Generated from Rust. Run pnpm ipc:generate.
export type OAuthStatus = {
    account_id: string;
    provider: string;
    connected: boolean;
    has_refresh_token: boolean;
    expires_at: string | null;
    granted_scopes: Array<string> | null;
};
