// Generated from Rust. Run pnpm ipc:generate.
/**
 * Keyset cursor over `(threads.last_message_date DESC, threads.id DESC)`.
 *
 * Offset pagination would skip or duplicate rows whenever a sync inserts a
 * thread between two page loads, which for a live mailbox is constant.
 */
export type ThreadCursor = {
    last_message_date: string;
    id: string;
};
