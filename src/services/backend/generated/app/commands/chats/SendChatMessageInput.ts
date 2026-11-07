// Generated from Rust. Run pnpm ipc:generate.
export type SendChatMessageInput = {
    thread_id: string;
    text: string;
    /**
     * Send encrypted without a signature although the own key is locked.
     * Only after the sender declined to unlock it and said so.
     */
    allow_unsigned: boolean;
    /**
     * The recipients the sender saw. A reply goes out only to exactly these.
     */
    expected_recipients: Array<string> | null;
    /**
     * The sender confirmed recipients that differ from the last reply.
     */
    confirm_recipients: boolean;
    /**
     * The sender confirmed sending in plain text in a chat that was encrypted.
     */
    allow_plaintext: boolean;
};
