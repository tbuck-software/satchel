// Generated from Rust. Run pnpm ipc:generate.
export type TopicTimelineItemDto = {
    item_type: string;
    item_id: string;
    thread_id: string | null;
    title: string | null;
    snippet: string | null;
    content: string | null;
    from_address: string | null;
    sort_date: string;
    added_at: string;
    added_by: string;
    message_count: number;
    unread_count: number;
};
