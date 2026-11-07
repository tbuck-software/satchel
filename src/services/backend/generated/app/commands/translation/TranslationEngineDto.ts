// Generated from Rust. Run pnpm ipc:generate.
export type TranslationEngineDto = {
    /**
     * `deepl`.
     */
    id: string;
    has_key: boolean;
    /**
     * DeepL: `free` or `pro`, read from the stored key's suffix.
     */
    plan: string | null;
    /**
     * False for every engine that sends text off the device. Shown only;
     * no setting depends on it yet (ADR 0016's "local models only" switch
     * does not exist).
     */
    local: boolean;
};
