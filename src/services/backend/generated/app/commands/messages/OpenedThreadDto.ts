// Generated from Rust. Run pnpm ipc:generate.
import type { MessageBodyDto } from "./MessageBodyDto";
import type { MessageDto } from "./MessageDto";
export type OpenedThreadDto = {
    messages: Array<MessageDto>;
    bodies: Array<MessageBodyDto>;
};
