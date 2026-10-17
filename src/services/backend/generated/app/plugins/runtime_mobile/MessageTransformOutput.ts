// Generated from Rust. Run pnpm ipc:generate.
export type MessageTransformOutput = {
    subject: string | null;
    snippet: string | null;
    body_text: string | null;
    body_html: string | null;
    labels: Array<string>;
    pgp_state: string | null;
    pgp_error_detail: string | null;
    pgp_signer_name: string | null;
    pgp_signer_fingerprint: string | null;
};
