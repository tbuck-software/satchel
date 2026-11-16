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
     * The guess says the body is not in the target language and nothing
     * refuses it: worth a button of its own.
     */
    offered: boolean;
    /**
     * A translation of the body as it is now, into the target language, is
     * stored on this device.
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
    /**
     * Why the active engine would not get this message, as a message key:
     * a plan that allows no personal data, encrypted, or no text.
     */
    refusal: string | null;
};
