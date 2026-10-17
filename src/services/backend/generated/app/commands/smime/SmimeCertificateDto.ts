// Generated from Rust. Run pnpm ipc:generate.
export type SmimeCertificateDto = {
    id: string;
    label: string;
    cert_type: string;
    fingerprint: string;
    subject_email: string | null;
    subject_cn: string | null;
    issuer_cn: string | null;
    not_before: string | null;
    not_after: string | null;
    has_passphrase: boolean;
    is_unlocked: boolean;
    created_at: string;
    updated_at: string;
};
