// Generated from Rust. Run pnpm ipc:generate.
export type RowIssue = {
    "kind": "invalid_email";
} | {
    "kind": "duplicate";
} | {
    "kind": "missing_values";
    fields: Array<string>;
};
