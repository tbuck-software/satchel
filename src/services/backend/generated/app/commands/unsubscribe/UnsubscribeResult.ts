// Generated from Rust. Run pnpm ipc:generate.
import type { UnsubscribeMail } from "./UnsubscribeMail";
export type UnsubscribeResult = {
    method: string;
    target: string;
    /**
     * Set when the next step is a mail: nothing was sent, and the caller
     * confirms this mail by passing it back as `confirmed_mail`.
     */
    confirm_mail: UnsubscribeMail | null;
};
