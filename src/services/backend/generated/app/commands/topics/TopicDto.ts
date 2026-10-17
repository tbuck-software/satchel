// Generated from Rust. Run pnpm ipc:generate.
export type TopicDto = {
    id: string;
    name: string;
    description: string;
    color: string | null;
    icon: string | null;
    status: string;
    due_date: string | null;
    item_count: number;
    note_count: number;
    unread_count: number;
    last_sender_name: string | null;
    last_sender_email: string | null;
    created_at: string;
    updated_at: string;
};
