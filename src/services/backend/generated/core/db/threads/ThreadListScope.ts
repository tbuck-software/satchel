// Generated from Rust. Run pnpm ipc:generate.
/**
 * Which thread list a thread belongs to: the contact, workflow state and
 * mailbox of a `ThreadListRequest`, without account, bucket or paging.
 *
 * Message search narrows to a list through this, so a search "in the Inbox"
 * finds what the Inbox shows — inbox *and* todo threads with a message in
 * the top-level inbox folder — and not what a single `in:` operator says.
 */
export type ThreadListScope = {
    contact_email?: string | null;
    mailbox_kind?: string | null;
    mailbox_value?: string | null;
    workflow_state?: string | null;
};
