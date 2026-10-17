// Generated from Rust. Run pnpm ipc:generate.
import type { SignatureAssetInput } from "./SignatureAssetInput";
export type SignatureInput = {
    id: string | null;
    account_id: string;
    name: string;
    body_text: string;
    body_html: string | null;
    short_body_text: string;
    short_body_html: string | null;
    assets: Array<SignatureAssetInput>;
    is_default: boolean;
};
