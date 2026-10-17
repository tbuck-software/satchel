// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
import type { OutgoingAttachmentInput } from "../messages/OutgoingAttachmentInput";
export type DraftInput = {
    /**
     * Optional id; if provided, an existing draft is updated.
     */
    id: string | null;
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
    /**
     * RFC3339; for Send Later draft.
     */
    scheduled_for: string | null;
    attachments: Array<OutgoingAttachmentInput>;
    signature_id: string | null;
    signature_variant: string | null;
    /**
     * Per-message edit of the resolved signature; None keeps the stored one.
     */
    signature_override_text: string | null;
    signature_override_html: string | null;
    send_options: JsonValue | null;
    /**
     * The `revision` this editor last saw. A mismatch means another window
     * changed, sent or discarded the draft, and the save is refused.
     */
    expected_revision?: number | null;
};
