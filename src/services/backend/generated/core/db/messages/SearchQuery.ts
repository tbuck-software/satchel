// Generated from Rust. Run pnpm ipc:generate.
import type { SearchCursor } from "./SearchCursor";
export type SearchQuery = {
    query: string;
    /**
     * Restrict to a single account. `None` searches every active account
     * (the unified-inbox case).
     */
    account_id?: string | null;
    /**
     * Result cap. Clamped to 1..=500; defaults to 50.
     */
    limit?: number | null;
    /**
     * Continue after the final hit of the previous page.
     */
    cursor?: SearchCursor | null;
};
