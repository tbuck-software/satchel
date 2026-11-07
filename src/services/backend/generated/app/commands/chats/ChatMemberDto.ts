// Generated from Rust. Run pnpm ipc:generate.
export type ChatMemberDto = {
    address: string;
    name: string | null;
    /**
     * Autocrypt recommendation for this member: `disable`, `discourage`,
     * `available` or `encrypt`.
     */
    encryption: string;
};
