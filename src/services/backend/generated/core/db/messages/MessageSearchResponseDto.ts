// Generated from Rust. Run pnpm ipc:generate.
import type { MessageSearchHitDto } from "./MessageSearchHitDto";
import type { SearchCursor } from "./SearchCursor";
import type { SearchWarningDto } from "./SearchWarningDto";
export type MessageSearchResponseDto = {
    hits: Array<MessageSearchHitDto>;
    warnings: Array<SearchWarningDto>;
    has_more: boolean;
    next_cursor: SearchCursor | null;
};
