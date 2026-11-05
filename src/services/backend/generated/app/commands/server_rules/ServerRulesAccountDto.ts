// Generated from Rust. Run pnpm ipc:generate.
export type ServerRulesAccountDto = {
    account_id: string;
    display_name: string;
    email: string;
    /**
     * `unknown`, `available` or `unavailable`.
     */
    availability: string;
    host: string | null;
    port: number | null;
    /**
     * `starttls` or `tls`.
     */
    tls: string | null;
    /**
     * Where the endpoint came from: `override`, `autoconfig` or `imap_host`.
     */
    source: string | null;
    host_override: string | null;
    port_override: number | null;
    tls_override: string | null;
    implementation: string | null;
    extensions: Array<string>;
    checked_at: string | null;
    enabled: boolean;
    server_active: boolean;
    /**
     * `direct` or `wrapper`.
     */
    activation_mode: string | null;
    previous_active_script: string | null;
    /**
     * Another script is active and a push waits for the person's choice.
     */
    consent_script: string | null;
    include_available: boolean;
    vacation_available: boolean;
    vacation_window_available: boolean;
    pushed_vacation: boolean;
    pushed_rule_count: number;
    last_push_at: string | null;
    /**
     * A message key.
     */
    last_error: string | null;
};
