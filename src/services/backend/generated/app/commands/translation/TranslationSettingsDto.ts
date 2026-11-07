// Generated from Rust. Run pnpm ipc:generate.
import type { TranslationConsentRecord } from "../../../core/db/translation/TranslationConsentRecord";
import type { TranslationEngineDto } from "./TranslationEngineDto";
export type TranslationSettingsDto = {
    /**
     * The engine translations go to; `None` while translation is off.
     */
    engine: string | null;
    /**
     * `None`: the webview's UI language.
     */
    target_language: string | null;
    log_sent_text: boolean;
    engines: Array<TranslationEngineDto>;
    consents: Array<TranslationConsentRecord>;
};
