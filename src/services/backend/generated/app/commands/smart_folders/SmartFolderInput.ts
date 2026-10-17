// Generated from Rust. Run pnpm ipc:generate.
export type SmartFolderInput = {
    id?: string | null;
    name: string;
    icon?: string | null;
    /**
     * Search query in the same syntax as `commands::messages::search_messages`
     * (e.g. "from:boss has:attachment in:inbox").
     */
    query: string;
    sort_order?: number | null;
};
