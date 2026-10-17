// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
import type { DiscoveredService } from "./DiscoveredService";
export type AccountDiscoveryDto = {
    ready: boolean;
    username: string;
    config: JsonValue;
    services: Array<DiscoveredService>;
    warnings: Array<string>;
};
