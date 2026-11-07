// Generated from Rust. Run pnpm ipc:generate.
export type WorkingHoursInput = {
    /**
     * Minutes after local midnight.
     */
    start_minute: number;
    /**
     * Minutes after local midnight, up to 1440.
     */
    end_minute: number;
    /**
     * ISO weekdays, 1 = Monday … 7 = Sunday.
     */
    weekdays: Array<number>;
};
