// Generated from Rust. Run pnpm ipc:generate.
import type { MergeRecipientInput } from "../../../core/mail_merge/MergeRecipientInput";
import type { OutgoingMessageInput } from "../messages/OutgoingMessageInput";
export type CreateMailMergeJobInput = {
    account_id: string;
    /**
     * The composer's send input. To, Cc and Bcc are ignored: every
     * recipient gets a copy addressed to them alone.
     */
    message: OutgoingMessageInput;
    source_label: string;
    delay_seconds: number;
    fields: Array<string>;
    email_field: string;
    rows: Array<MergeRecipientInput>;
    /**
     * The client's token for this start. Sent again after a lost answer, it
     * returns the job already made instead of a second one.
     */
    request_id?: string | null;
};
