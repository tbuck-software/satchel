// Generated from Rust. Run pnpm ipc:generate.
import type { SignatureAssetDto } from "./SignatureAssetDto";
export type SignatureDto = {
    id: string;
    account_id: string;
    name: string;
    body_text: string;
    body_html: string | null;
    short_body_text: string;
    short_body_html: string | null;
    assets: Array<SignatureAssetDto>;
    is_default: number;
    created_at: string;
    updated_at: string;
};
