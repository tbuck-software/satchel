// Generated from Rust. Run pnpm ipc:generate.
/**
 * A sender in the cleanup view: how much Inbox mail they sent lately.
 */
export type CleanupSender = {
    account_id: string;
    sender_email: string;
    sender_name: string | null;
    /**
     * Messages from the sender inside the window whose thread is in the Inbox.
     */
    message_count: number;
    unread_count: number;
    /**
     * Inbox threads those messages belong to.
     */
    thread_count: number;
    newest_date: string;
    /**
     * The newest message inside the window with a `List-Unsubscribe` header.
     */
    unsubscribe_message_id: string | null;
    /**
     * The recurring rule for this sender, if one exists.
     */
    rule_id: string | null;
};
