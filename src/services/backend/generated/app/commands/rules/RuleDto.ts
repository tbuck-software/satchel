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
    /**
     * The one account the rule applies to; `None` for every account. A rule
     * that moves mail names the account whose folder it moves to.
     */
    account_id: string | null;
    /**
     * Set when the rule forwarded more than allowed within an hour; its
     * forward stays off until the person resumes it (`resume_rule_forwarding`).
     */
    forward_paused_at: string | null;
    /**
     * Forwards left to the server because the rule ran there when the mail
     * arrived.
     */
    server_forward_skips: number;
};
