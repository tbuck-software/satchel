// Generated from Rust. Run pnpm ipc:generate.
import type { OutgoingAttachment } from "../../../core/providers/traits/OutgoingAttachment";
import type { SignatureVariant } from "./SignatureVariant";
export type ResolvedSignature = {
    signature_id: string;
    variant: SignatureVariant;
    body_text: string;
    body_html: string | null;
    attachments: Array<OutgoingAttachment>;
};
