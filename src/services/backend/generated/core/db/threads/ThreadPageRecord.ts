// Generated from Rust. Run pnpm ipc:generate.
import type { ThreadCursor } from "./ThreadCursor";
import type { ThreadRecord } from "./ThreadRecord";
export type ThreadPageRecord = {
    threads: Array<ThreadRecord>;
    /**
     * Pass back as `cursor` to fetch the next page. `None` = end of list.
     */
    next_cursor: ThreadCursor | null;
    has_more: boolean;
};
