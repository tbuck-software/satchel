// Generated from Rust. Run pnpm ipc:generate.
import type { OutgoingAttachmentInput } from "./OutgoingAttachmentInput";
export type OutgoingMessageInput = {
    to: Array<string>;
    cc: Array<string>;
    bcc: Array<string>;
    subject: string;
    body: string;
    body_html: string | null;
    thread_id: string | null;
    /**
     * Local id of the message this one answers. The thread alone does not say
     * which branch the user replied to, and the newest message in it is often
     * not the one they had open.
     */
    reply_to_message_id: string | null;
    attachments: Array<OutgoingAttachmentInput>;
    /**
     * Override the From-header (for aliases / send-as). `send_message` rejects
     * anything that is neither the account's primary address nor one of its
     * aliases. None means "use account primary address".
     */
    from_address: string | null;
    from_name: string | null;
    reply_to: string | null;
    /**
     * Local draft id; if set the draft row will be deleted on successful send.
     */
    draft_id: string | null;
    /**
     * RFC3339 timestamp at which to send (Send Later). None = send now.
     */
    scheduled_for: string | null;
    /**
     * RFC3339 timestamp for "remind me if no reply".
     */
    follow_up_at: string | null;
    smime_sign_with_cert_id: string | null;
    smime_encrypt_to_recipients: boolean;
    pgp_sign_with_key_id: string | null;
    pgp_encrypt_to_recipients: boolean;
    /**
     * "high" | "normal" | "low". Normal/None sends no header at all.
     */
    priority: string | null;
    signature_id: string | null;
    signature_variant: string | null;
    /**
     * Per-message edit of the resolved signature; replaces the stored body
     * while keeping the signature's identity and inline assets.
     */
    signature_override_text: string | null;
    signature_override_html: string | null;
};
