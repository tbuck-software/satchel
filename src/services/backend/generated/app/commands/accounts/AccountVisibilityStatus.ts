// Generated from Rust. Run pnpm ipc:generate.
import type { AccountVisibilityPolicy } from "../../../core/db/account_visibility/AccountVisibilityPolicy";
export type AccountVisibilityStatus = {
    account_id: string;
    policy: AccountVisibilityPolicy;
    visible: boolean;
    local_date: string;
};
