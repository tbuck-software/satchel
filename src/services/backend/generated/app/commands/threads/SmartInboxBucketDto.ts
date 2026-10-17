// Generated from Rust. Run pnpm ipc:generate.
import type { SmartInboxPreviewDto } from "./SmartInboxPreviewDto";
export type SmartInboxBucketDto = {
    key: string;
    count: number;
    unread_count: number;
    preview_items: Array<SmartInboxPreviewDto>;
};
