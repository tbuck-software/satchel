// Generated from Rust. Run pnpm ipc:generate.
export type UpdateThreadOrganizationInput = {
    thread_id: string;
    board_position?: number | null;
    assignee?: string | null;
    shared_with: Array<string>;
};
