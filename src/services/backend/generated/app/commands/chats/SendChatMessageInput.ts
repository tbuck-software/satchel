// Generated from Rust. Run pnpm ipc:generate.
export type SendChatMessageInput = {
    thread_id: string;
    text: string;
    /**
     * Send encrypted without a signature although the own key is locked.
     * Only after the sender declined to unlock it and said so.
     */
    allow_unsigned: boolean;
};
