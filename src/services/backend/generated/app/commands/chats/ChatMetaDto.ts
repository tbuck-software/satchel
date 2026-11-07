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
     * The own key's id, for unlocking it before an encrypted reply.
     */
    own_key_id: string | null;
    /**
     * Whether the own key is unlocked this session, so it can sign.
     */
    own_key_unlocked: boolean;
    /**
     * Whether the next message will be end-to-end encrypted.
     */
    encrypts: boolean;
    /**
     * Who the user's last reply went to, if there was one.
     */
    last_reply_recipients: Array<string> | null;
    /**
     * The members differ from the last reply; the reply field asks first.
     */
    recipients_changed: boolean;
    /**
     * The chat was encrypted and the next message would not be; the reply
     * field asks first.
     */
    encryption_downgrade: boolean;
};
