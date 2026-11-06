// Generated from Rust. Run pnpm ipc:generate.
import type { TaskFieldsInput } from "./TaskFieldsInput";
export type CreateTaskInput = {
    list_id: string;
    fields: TaskFieldsInput;
    /**
     * The thread the task is made from; its newest Message-ID is stored too.
     */
    thread_id?: string | null;
};
