// Generated from Rust. Run pnpm ipc:generate.
/**
 * What a backup says about itself, for listing it without opening it.
 *
 * The counts are taken once, while the backup is being written. Reading them
 * back out of the database would mean opening every listed file.
 */
export type BackupManifest = {
    schema_version: number;
    app_version: string;
    created_at: string;
    account_count: number;
    message_count: number;
    /**
     * Whether the search index matched the message rows when the backup was
     * written. `None` in a manifest from before this field — unknown is not
     * the same as broken.
     */
    search_index_ok: boolean | null;
};
