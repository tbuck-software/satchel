// Generated from Rust. Run pnpm ipc:generate.
/**
 * An account that keeps Done and Snooze on this device, and why:
 * `no_custom_keywords` (the server keeps no keywords of its own),
 * `gmail_imap` (Gmail over IMAP) or `store_refused` (the server refused the
 * last write; tried again later).
 */
export type LocalOnlyWorkflowAccountDto = {
    account_id: string;
    reason: string;
};
