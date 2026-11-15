// Generated from Rust. Run pnpm ipc:generate.
import type { UnsubscribeMail } from "./UnsubscribeMail";
export type UnsubscribeInput = {
    message_id: string;
    /**
     * The mail the user confirmed. It is sent only if it still equals what
     * the message's header yields; nothing else in the plan runs.
     */
    confirmed_mail?: UnsubscribeMail | null;
};
