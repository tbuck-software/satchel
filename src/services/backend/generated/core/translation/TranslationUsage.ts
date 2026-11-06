// Generated from Rust. Run pnpm ipc:generate.
import type { UsageUnit } from "./UsageUnit";
/**
 * How much of the account's allowance this billing period has used.
 */
export type TranslationUsage = {
    used: number;
    /**
     * `None`: no limit is set.
     */
    limit: number | null;
    unit: UsageUnit;
};
