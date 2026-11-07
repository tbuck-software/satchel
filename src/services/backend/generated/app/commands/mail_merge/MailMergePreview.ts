// Generated from Rust. Run pnpm ipc:generate.
import type { MergePlan } from "../../../core/mail_merge/MergePlan";
import type { RenderedMessage } from "../../../core/mail_merge/RenderedMessage";
export type MailMergePreview = {
    plan: MergePlan;
    rendered: RenderedMessage | null;
    /**
     * Fields that keep the previewed row from being sent.
     */
    missing: Array<string>;
    max_recipients: number;
    /**
     * The pace that suits the account's provider; 0 without an account.
     */
    default_delay_seconds: number;
};
