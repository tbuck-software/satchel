// Generated from Rust. Run pnpm ipc:generate.
import type { TaskFieldsInput } from "./TaskFieldsInput";
export type UpdateTaskInput = {
    id: string;
    /**
     * The revision the editor opened; a newer one (a sync) wins.
     */
    expected_updated_at?: string | null;
    fields: TaskFieldsInput;
};
