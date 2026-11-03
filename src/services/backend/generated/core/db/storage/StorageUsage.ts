// Generated from Rust. Run pnpm ipc:generate.
export type StorageUsage = {
    /**
     * SQLite's logical file size, including pages currently in the WAL.
     */
    database_bytes: number;
    reusable_bytes: number;
    wal_bytes: number;
    /**
     * A cache target, never permission to delete headers or unique local data.
     */
    target_bytes: number | null;
};
