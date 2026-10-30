// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
import type { OutgoingAttachmentInput } from "../messages/OutgoingAttachmentInput";
export type DraftDto = {
    id: string;
    account_id: string;
    thread_id: string | null;
    to: Array<string>;
    cc: Array<string>;
    bcc: Array<string>;
    subject: string;
    body: string;
    body_html: string | null;
    from_address: string | null;
    from_name: string | null;
    reply_to: string | null;
    scheduled_for: string | null;
    attachments: Array<OutgoingAttachmentInput>;
    signature_id: string | null;
    signature_variant: string | null;
    signature_override_text: string | null;
    signature_override_html: string | null;
    updated_at: string;
    /**
     * Bumped by every draft save; editors send it back as `expected_revision`.
     */
    revision: number;
    /**
     * Set when the draft is on disk but the provider copy is not. The draft
     * itself is valid — this only says the remote half has to wait.
     */
    remote_error: string | null;
    /**
     * Encoded recovery key and parameters when this row's content cannot be
     * trusted. Only identifiers and timestamps remain; load still fails closed.
     */
    content_error: string | null;
    send_cancelled: boolean;
    send_options: JsonValue | null;
};
