// Generated from Rust. Run pnpm ipc:generate.
export type StartOAuthResponse = {
    authorization_url: string;
    state: string;
    expires_at: string;
    browser_opened: boolean;
    browser_error: string | null;
};
