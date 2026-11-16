// Generated from Rust. Run pnpm ipc:generate.
import type { MailMergeJobSummary } from "./MailMergeJobSummary";
import type { MailMergeRecipientRow } from "./MailMergeRecipientRow";
export type MailMergeJobDetail = {
    job: MailMergeJobSummary;
    recipients: Array<MailMergeRecipientRow>;
};
