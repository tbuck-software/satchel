// Generated from Rust. Run pnpm ipc:generate.
export type EnableServerRulesInput = {
    account_id: string;
    /**
     * `ask` never takes over another active script; `wrapper` and
     * `replace` are the person's explicit answer to that question.
     */
    choice: string;
    /**
     * The active script the person saw when choosing. If another one is
     * active by now, the push stops and asks again.
     */
    expected_active?: string | null;
};
