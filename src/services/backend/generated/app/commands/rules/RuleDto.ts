// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
export type RuleDto = {
    id: string;
    name: string;
    description: string | null;
    trigger_type: string;
    trigger_config: JsonValue;
    actions: Array<string>;
    is_active: boolean;
    priority: number;
};
