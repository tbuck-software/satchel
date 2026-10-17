// Generated from Rust. Run pnpm ipc:generate.
import type { OAuthStatus } from "./OAuthStatus";
export type OAuthFlowPollResponse = {
    status: string;
    result: OAuthStatus | null;
    error: string | null;
};
