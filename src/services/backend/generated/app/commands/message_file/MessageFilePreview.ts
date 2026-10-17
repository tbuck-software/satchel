// Generated from Rust. Run pnpm ipc:generate.
import type { MessageFileAttachment } from "./MessageFileAttachment";
export type MessageFilePreview = {
    path: string;
    subject: string;
    from_address: string;
    from_name: string | null;
    to_addresses: Array<string>;
    cc_addresses: Array<string>;
    date_rfc3339: string;
    body_html: string | null;
    body_text: string | null;
    attachments: Array<MessageFileAttachment>;
};
