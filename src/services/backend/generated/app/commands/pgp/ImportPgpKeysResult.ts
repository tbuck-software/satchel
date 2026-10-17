// Generated from Rust. Run pnpm ipc:generate.
import type { ImportPgpKeyFailure } from "./ImportPgpKeyFailure";
import type { PgpKeyDto } from "./PgpKeyDto";
export type ImportPgpKeysResult = {
    imported: Array<PgpKeyDto>;
    skipped: number;
    failed: number;
    failures: Array<ImportPgpKeyFailure>;
    detected: number;
};
