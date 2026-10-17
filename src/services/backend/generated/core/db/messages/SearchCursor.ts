// Generated from Rust. Run pnpm ipc:generate.
export type SearchCursor = {
    rank_cost: number;
    date: string;
    /**
     * Thread id breaks ties after the best matching message is selected.
     */
    id: string;
};
