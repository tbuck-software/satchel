// Generated from Rust. Run pnpm ipc:generate.
export type PgpKeyDto = {
    id: string;
    label: string;
    key_type: string;
    fingerprint: string;
    user_ids: Array<string>;
    has_passphrase: boolean;
    is_unlocked: boolean;
    is_expired: boolean;
    can_encrypt: boolean;
    created_at: string;
    updated_at: string;
};
