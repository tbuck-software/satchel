// Generated from Rust. Run pnpm ipc:generate.
export type CertificateExceptionDto = {
    account_id: string;
    host: string;
    fingerprint: string;
    /**
     * RFC 3339.
     */
    expires_at: string;
    /**
     * RFC 3339.
     */
    created_at: string;
};
