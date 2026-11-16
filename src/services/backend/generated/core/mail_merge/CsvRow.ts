// Generated from Rust. Run pnpm ipc:generate.
export type CsvRow = {
    /**
     * 1-based line in the file where the row starts, for "row 14 is invalid".
     */
    line: number;
    values: {
        [key in string]: string;
    };
};
