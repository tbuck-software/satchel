// Generated from Rust. Run pnpm ipc:generate.
import type { GroupwareSyncSummary } from "../../../core/providers/dav/GroupwareSyncSummary";
export type DavAutoconfigureDto = {
    caldav_server: string | null;
    carddav_server: string | null;
    summary: GroupwareSyncSummary;
};
