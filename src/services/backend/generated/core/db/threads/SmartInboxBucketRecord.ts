// Generated from Rust. Run pnpm ipc:generate.
import type { SmartInboxPreviewRecord } from "./SmartInboxPreviewRecord";
export type SmartInboxBucketRecord = {
    key: string;
    count: number;
    unread_count: number;
    preview_items: Array<SmartInboxPreviewRecord>;
};
