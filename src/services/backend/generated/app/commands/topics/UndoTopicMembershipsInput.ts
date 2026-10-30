// Generated from Rust. Run pnpm ipc:generate.
import type { TopicMembershipReceiptDto } from "./TopicMembershipReceiptDto";
export type UndoTopicMembershipsInput = {
    topic_id: string;
    item_type: string;
    receipts: Array<TopicMembershipReceiptDto>;
    restore: boolean;
};
