// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
export type NewAccount = {
    /**
     * The sender name on outgoing mail.
     */
    display_name: string;
    /**
     * Satchel's own name for the account, never sent.
     */
    label?: string | null;
    email: string;
    provider: string;
    config?: JsonValue | null;
};
