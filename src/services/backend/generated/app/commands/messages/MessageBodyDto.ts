// Generated from Rust. Run pnpm ipc:generate.
import type { BodyProvenance } from "./BodyProvenance";
import type { BodySegment } from "./BodySegment";
export type MessageBodyDto = {
    id: string;
    remote_uidvalidity?: number | null;
    /**
     * A remote identity that cannot serve content until its folder is synced.
     */
    content_error?: string | null;
    snippet: string;
    body_text: string;
    body_segments: Array<BodySegment> | null;
    body_provenance: BodyProvenance | null;
    body_html: string | null;
    body_loaded: boolean;
    pgp_state: string | null;
    pgp_error_detail: string | null;
    pgp_signer_name: string | null;
    pgp_signer_fingerprint: string | null;
    smime_state: string | null;
    smime_signer_email: string | null;
    smime_signer_cn: string | null;
    smime_error_detail: string | null;
    /**
     * The headers behind the unsubscribe banner. The preview leaves them
     * empty; the banner sits beside a loaded body, so the body carries them.
     */
    list_unsubscribe: string | null;
    list_unsubscribe_post: string | null;
    auth_spf: string | null;
    auth_dkim: string | null;
    auth_dmarc: string | null;
    /**
     * Structured item data for source messages (feeds, webhooks). Sent with
     * the body rather than the preview: it is only rendered beside a body.
     */
    source_payload: string | null;
};
