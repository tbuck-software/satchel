// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
export type IngestItem = {
    external_id: string | null;
    thread_key: string | null;
    title: string;
    author: string | null;
    body: string | null;
    body_html: string | null;
    timestamp: string | null;
    level: string | null;
    url: string | null;
    data: JsonValue | null;
};
