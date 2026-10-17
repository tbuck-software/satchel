// Generated from Rust. Run pnpm ipc:generate.
import type { ThreadCursor } from "./ThreadCursor";
export type ThreadListRequest = {
    account_id: string | null;
    contact_email: string | null;
    mailbox_kind: string | null;
    mailbox_value: string | null;
    workflow_state: string | null;
    /**
     * Focused-list bucket. Applied server-side so opening a group can page
     * through all of its threads instead of only the current inbox page.
     */
    smart_bucket: string | null;
    /**
     * Keyset cursor from a previous page. `None` starts at the newest thread.
     */
    cursor: ThreadCursor | null;
    /**
     * Fetch the nearest newer rows, returned in normal descending order.
     */
    before_cursor: ThreadCursor | null;
    /**
     * Page size. Clamped to 1..=1000; defaults to `thread_page_size()`.
     */
    limit: number | null;
};
