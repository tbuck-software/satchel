// Generated from Rust. Run pnpm ipc:generate.
import type { BodySegment } from "./BodySegment";
import type { DecryptedAttachmentBinding } from "./DecryptedAttachmentBinding";
/**
 * Exact transient render inputs issued together after decoding and sanitizing.
 * PR54 integration must bind this snapshot and attachment publication to the
 * same saved ContentReference, with generation checks around every await.
 */
export type BodyProvenance = {
    segments: Array<BodySegment>;
    html: string | null;
    attachments: Array<DecryptedAttachmentBinding>;
};
