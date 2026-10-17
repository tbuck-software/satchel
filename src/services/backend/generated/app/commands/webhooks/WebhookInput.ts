// Generated from Rust. Run pnpm ipc:generate.
export type WebhookInput = {
    id?: string | null;
    url: string;
    events: Array<string>;
    enabled: boolean;
    secret?: string | null;
};
