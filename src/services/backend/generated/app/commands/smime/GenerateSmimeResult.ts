// Generated from Rust. Run pnpm ipc:generate.
import type { SmimeCertificateDto } from "./SmimeCertificateDto";
export type GenerateSmimeResult = {
    certificate: SmimeCertificateDto;
    pem_certificate: string;
    fingerprint: string;
};
