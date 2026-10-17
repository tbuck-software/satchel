// Generated from Rust. Run pnpm ipc:generate.
/**
 * An operator token the parser understood but had to ignore, e.g. `in:` with
 * an unknown mailbox or an unparseable date. Machine-readable on purpose —
 * the user-facing (German) wording lives in the frontend.
 */
export type SearchWarningDto = {
    operator: string;
    value: string;
};
