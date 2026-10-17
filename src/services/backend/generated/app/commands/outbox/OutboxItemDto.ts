// Generated from Rust. Run pnpm ipc:generate.
import type { OutboxAttachmentDto } from "./OutboxAttachmentDto";
export type OutboxItemDto = {
    id: string;
    account_id: string;
    action_type: string;
    status: string;
    retry_count: number;
    created_at: string;
    updated_at: string;
    last_error: string | null;
    next_retry_at: string | null;
    scheduled_for: string | null;
    subject: string | null;
    recipients: Array<string>;
    cc: Array<string>;
    bcc: Array<string>;
    /**
     * Enough of the payload to rebuild the message in the composer. Without it
     * a mail that never went out leaves nothing but its subject.
     */
    body: string | null;
    body_html: string | null;
    attachments: Array<OutboxAttachmentDto>;
    /**
     * Set once the provider has seen bytes of this attempt.
     */
    submission_started_at: string | null;
};
