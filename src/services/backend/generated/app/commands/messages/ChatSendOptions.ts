// Generated from Rust. Run pnpm ipc:generate.
/**
 * What the outgoing message needs besides the usual headers.
 */
export type ChatSendOptions = {
    group_id?: string | null;
    group_name?: string | null;
    /**
     * The sender chose to send encrypted but unsigned because the own key
     * stays locked. Without it a locked key holds the reply back.
     */
    allow_unsigned: boolean;
    /**
     * The reply was end-to-end encrypted when the sender sent it. A retry
     * whose plan no longer encrypts (a peer key or the own key gone) waits
     * in the Outbox instead of going out in plain text.
     */
    encryption_required: boolean;
};
