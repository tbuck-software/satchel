// Generated from Rust. Run pnpm ipc:generate.
export type ImportIcsInput = {
    /**
     * Optional account_id to anchor the invite to. If omitted, the first active
     * account is used (works fine for the common single-account case).
     */
    account_id?: string | null;
    /**
     * Either the raw ICS text or a base64-encoded file content.
     */
    raw: string;
    source_filename?: string | null;
};
