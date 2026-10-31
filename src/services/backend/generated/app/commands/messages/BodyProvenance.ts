// Generated from Rust. Run pnpm ipc:generate.
import type { BodySegment } from "./BodySegment";
import type { DecryptedAttachmentBinding } from "./DecryptedAttachmentBinding";
/**
 * Exact transient render inputs issued together after decoding and
 * sanitizing. The renderer grants captions only while the body matches this
 * snapshot.
 */
export type BodyProvenance = {
    segments: Array<BodySegment>;
    html: string | null;
    attachments: Array<DecryptedAttachmentBinding>;
};
