// Generated from Rust. Run pnpm ipc:generate.
import type { ChatMemberDto } from "./ChatMemberDto";
export type ChatMetaDto = {
    thread_id: string;
    account_id: string;
    own_address: string;
    group_id: string | null;
    group_name: string | null;
    /**
     * The other people in the chat, in header order.
     */
    members: Array<ChatMemberDto>;
    /**
     * Whether this account has an OpenPGP key for `own_address`, which
     * Autocrypt needs to announce and encrypt.
     */
    own_key: boolean;
    /**
     * Whether the next message will be end-to-end encrypted.
     */
    encrypts: boolean;
};
