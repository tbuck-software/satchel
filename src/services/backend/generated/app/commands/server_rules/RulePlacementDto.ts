// Generated from Rust. Run pnpm ipc:generate.
import type { LocalReasonDto } from "./LocalReasonDto";
export type RulePlacementDto = {
    rule_id: string;
    server_account_ids: Array<string>;
    local_reason: LocalReasonDto | null;
    /**
     * Compilable, but the server does not run this version yet.
     */
    pending: boolean;
};
