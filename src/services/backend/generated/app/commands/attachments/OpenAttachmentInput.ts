// Generated from Rust. Run pnpm ipc:generate.
export type OpenAttachmentInput = {
    attachment_id: string;
    /**
     * Set once the user has seen the full file name — both extensions — and
     * asked for it anyway. Absent, an executable type is refused rather than
     * handed to the operating system.
     */
    confirmed_executable: boolean;
};
