// Generated from Rust. Run pnpm ipc:generate.
export type SetSenderPreferenceInput = {
    account_id: string;
    sender_email: string;
    /**
     * Missing means unchanged; JSON null clears the account's category.
     */
    category?: string | null;
    is_priority?: boolean | null;
};
