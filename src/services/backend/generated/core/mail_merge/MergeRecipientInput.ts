// Generated from Rust. Run pnpm ipc:generate.
export type MergeRecipientInput = {
    /**
     * Where the row came from: a CSV line, or none for a contact.
     */
    line?: number | null;
    values: {
        [key in string]: string;
    };
};
