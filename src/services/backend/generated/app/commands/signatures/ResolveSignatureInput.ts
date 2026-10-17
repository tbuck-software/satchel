// Generated from Rust. Run pnpm ipc:generate.
import type { SignaturePreference } from "./SignaturePreference";
export type ResolveSignatureInput = {
    account_id: string;
    identity_email: string;
    thread_id: string | null;
    signature_id: string | null;
    preference: SignaturePreference;
};
