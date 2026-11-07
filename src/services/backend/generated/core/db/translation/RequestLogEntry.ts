// Generated from Rust. Run pnpm ipc:generate.
/**
 * One request that left the device.
 */
export type RequestLogEntry = {
    id: number;
    /**
     * RFC 3339.
     */
    created_at: string;
    /**
     * `translation`; later the AI features of ADR 0016.
     */
    feature: string;
    /**
     * The engine or provider, e.g. `deepl`.
     */
    provider: string;
    model: string | null;
    account_id: string | null;
    message_id: string | null;
    /**
     * What the provider billed, in `unit`.
     */
    units: number;
    /**
     * `characters` or `tokens`.
     */
    unit: string;
    /**
     * `ok`, or the message key the request ended in.
     */
    outcome: string;
    /**
     * The sent text was kept (the person asked for that). It is loaded one
     * entry at a time with [`TranslationRepository::sent_text`].
     */
    has_sent_text: boolean;
};
