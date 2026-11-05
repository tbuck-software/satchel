// Generated from Rust. Run pnpm ipc:generate.
export type ServerRulesEndpointInput = {
    account_id: string;
    /**
     * Empty or missing clears the override.
     */
    host?: string | null;
    port?: number | null;
    /**
     * `starttls` or `tls`.
     */
    tls?: string | null;
};
