// Generated from Rust. Run pnpm ipc:generate.
export type BusyBlock = {
    /**
     * RFC 3339, UTC.
     */
    starts_at: string;
    ends_at: string;
    /**
     * Accepted with a maybe, or marked tentative by the provider.
     */
    tentative: boolean;
};
