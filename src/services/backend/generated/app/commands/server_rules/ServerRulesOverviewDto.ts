// Generated from Rust. Run pnpm ipc:generate.
import type { RulePlacementDto } from "./RulePlacementDto";
import type { ServerRulesAccountDto } from "./ServerRulesAccountDto";
export type ServerRulesOverviewDto = {
    accounts: Array<ServerRulesAccountDto>;
    rules: Array<RulePlacementDto>;
};
