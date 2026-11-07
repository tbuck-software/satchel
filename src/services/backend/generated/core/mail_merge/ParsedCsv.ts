// Generated from Rust. Run pnpm ipc:generate.
import type { CsvEncoding } from "./CsvEncoding";
import type { CsvRow } from "./CsvRow";
export type ParsedCsv = {
    fields: Array<string>;
    rows: Array<CsvRow>;
    /**
     * The column that holds addresses, when one could be told from the
     * header or the values. The user can pick another.
     */
    email_field: string | null;
    /**
     * `,`, `;` or a tab.
     */
    delimiter: string;
    encoding: CsvEncoding;
    /**
     * More rows than [`MAX_CSV_ROWS`] were in the file; the rest was not read.
     */
    truncated: boolean;
};
