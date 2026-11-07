// Generated from Rust. Run pnpm ipc:generate.
import type { LocalReasonDto } from "./LocalReasonDto";
export type ActionPlacementDto = {
    /**
     * `server`: runs on the server of every account the rule applies to.
     * `pending`: could, but this version is not verified there yet.
     * `local`: runs in Satchel only, for `reason`.
     */
    state: string;
    reason: LocalReasonDto | null;
};
