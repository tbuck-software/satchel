// Generated from Rust. Run pnpm ipc:generate.
export type VacationInput = {
    account_id: string;
    enabled: boolean;
    subject: string | null;
    body: string;
    /**
     * RFC3339; vacation reply only fires while now() is in [starts_at, ends_at).
     */
    starts_at: string | null;
    ends_at: string | null;
    /**
     * If true: only reply to known contacts (Gmail-specific).
     */
    only_contacts: boolean;
};
