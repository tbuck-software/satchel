// Generated from Rust. Run pnpm ipc:generate.
export type BackupEntry = {
    path: string;
    /**
     * Directory name, `satchel-YYYYMMDD-HHMMSS`.
     */
    name: string;
    created_at: string;
    bytes: number;
    account_count: number;
    message_count: number;
    /**
     * Number of exported keys or local keychain snapshots. Zero means encrypted
     * credentials cannot be read back.
     */
    master_key_files: number;
    /**
     * The key travels wrapped under a passphrase, so restoring needs it.
     */
    master_key_wrapped: boolean;
    /**
     * Highest migration version in the backup. Above what this build knows,
     * `restore_backup` refuses it — knowing that before choosing helps.
     */
    schema_version: number;
    /**
     * False when the backup's search index no longer matched its messages when
     * it was written. `None` for a backup without a manifest: the listing does
     * not open the database, so it cannot tell. `restore_backup` checks.
     */
    search_index_ok: boolean | null;
};
