// Generated from Rust. Run pnpm ipc:generate.
import type { ServerOverride } from "./ServerOverride";
export type DiscoverAccountInput = {
    email: string;
    password: string;
    /**
     * "imap" (default) or "pop3".
     */
    protocol?: string | null;
    username?: string | null;
    imap?: ServerOverride | null;
    pop3?: ServerOverride | null;
    smtp?: ServerOverride | null;
};
