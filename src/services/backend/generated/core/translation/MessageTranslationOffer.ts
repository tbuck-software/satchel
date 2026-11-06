// Generated from Rust. Run pnpm ipc:generate.
/**
 * Whether Translate is worth offering for a message, without sending it.
 */
export type MessageTranslationOffer = {
    /**
     * The local guess at the body's language, ISO 639-1 upper case.
     */
    detected_language: string | null;
    /**
     * False when the guess says the body already is in the target language,
     * or there is nothing that could be sent.
     */
    offered: boolean;
    /**
     * A translation into the target language is stored on this device.
     */
    cached: boolean;
    /**
     * The cached translation's source language as the engine reported it.
     */
    cached_source_language: string | null;
    /**
     * The message's account may send mail to the active engine.
     */
    consented: boolean;
};
