// Generated from Rust. Run pnpm ipc:generate.
import type { PlannedRow } from "./PlannedRow";
export type MergePlan = {
    rows: Array<PlannedRow>;
    /**
     * Rows that will be sent.
     */
    ready: number;
    /**
     * Placeholders naming no field of the list. They render as their
     * fallback or block the row, so a typo shows up before sending.
     */
    unknown_fields: Array<string>;
};
