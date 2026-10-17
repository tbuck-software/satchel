// Generated from Rust. Run pnpm ipc:generate.
export type InviteResponse = {
    invite_id: string;
    /**
     * "accepted" | "declined" | "tentative"
     */
    response: string;
    comment?: string | null;
};
