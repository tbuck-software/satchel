// Generated from Rust. Run pnpm ipc:generate.
import type { JsonValue } from "../../../serde_json/JsonValue";
/**
 * Error contract at a Tauri command boundary.
 *
 * Rust chooses the message key and supplies interpolation values. The webview
 * owns the language and turns this object into the text shown to the user.
 */
export type CommandError = {
    key: string;
    params?: {
        [key in string]: JsonValue;
    };
};
