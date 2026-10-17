// Generated from Rust. Run pnpm ipc:generate.
import type { PgpKeyDto } from "./PgpKeyDto";
export type GeneratePgpKeyResult = {
    key: PgpKeyDto;
    public_key: string;
    fingerprint: string;
};
