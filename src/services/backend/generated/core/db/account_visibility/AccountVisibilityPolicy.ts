// Generated from Rust. Run pnpm ipc:generate.
export type AccountVisibilityPolicy = {
    mode: string;
    weekdays: Array<number>;
    from: string | null;
    to: string | null;
    override_date: string | null;
    override_visible: boolean | null;
};
