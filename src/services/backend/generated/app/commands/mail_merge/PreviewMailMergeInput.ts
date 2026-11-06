// Generated from Rust. Run pnpm ipc:generate.
import type { MergeRecipientInput } from "../../../core/mail_merge/MergeRecipientInput";
import type { MergeTemplate } from "../../../core/mail_merge/MergeTemplate";
export type PreviewMailMergeInput = {
    account_id?: string | null;
    template: MergeTemplate;
    fields: Array<string>;
    email_field: string;
    rows: Array<MergeRecipientInput>;
    /**
     * The row to render; out of range renders nothing.
     */
    index?: number | null;
};
