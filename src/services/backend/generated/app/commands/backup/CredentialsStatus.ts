// Generated from Rust. Run pnpm ipc:generate.
export type CredentialsStatus = {
    total_accounts: number;
    encrypted_accounts: number;
    plaintext_accounts: number;
    master_key_present: boolean;
    database_bytes: number;
    /**
     * A restore is staged and takes effect on the next start.
     */
    restore_pending: boolean;
};
