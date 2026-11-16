// Generated from Rust. Run pnpm ipc:generate.
import type { RowIssue } from "./RowIssue";
export type PlannedRow = {
    /**
     * Index in the input list.
     */
    index: number;
    line?: number | null;
    /**
     * The normalized address, or the raw cell when it is not one.
     */
    email: string;
    issue?: RowIssue | null;
};
