// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
export type CreateSourceInput = {
    name: string;
    kind: string;
    identity: string;
    color?: string | null;
    config: JsonValue;
};
