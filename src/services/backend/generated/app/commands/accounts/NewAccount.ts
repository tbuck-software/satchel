// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
export type NewAccount = {
    display_name: string;
    email: string;
    provider: string;
    config?: JsonValue | null;
};
