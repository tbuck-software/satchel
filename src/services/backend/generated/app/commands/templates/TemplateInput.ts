// Generated from Rust. Run pnpm ipc:generate.
export type TemplateInput = {
    id: string | null;
    account_id: string | null;
    name: string;
    subject: string | null;
    body_text: string;
    body_html: string | null;
    shortcut: string | null;
    cc: Array<string>;
    bcc: Array<string>;
    attachments_json: string | null;
};
