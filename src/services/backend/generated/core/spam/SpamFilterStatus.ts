// Generated from Rust. Run pnpm ipc:generate.
import type { SpamFilterMode } from "./SpamFilterMode";
/**
 * What the settings page shows about the filter.
 */
export type SpamFilterStatus = {
    mode: SpamFilterMode;
    spam_messages: number;
    ham_messages: number;
    tokens: number;
    /**
     * Whether the filter gives verdicts at all.
     */
    ready: boolean;
    /**
     * The one-time pass over existing mail has not finished yet.
     */
    bootstrap_pending: boolean;
    /**
     * More examples of each class it needs before it gives verdicts: the
     * floor of [`classifier::MIN_TRAINED_PER_CLASS`] and the class ratio.
     */
    spam_needed: number;
    ham_needed: number;
    /**
     * Whether Move may move mail; until then Move behaves like Mark.
     */
    move_ready: boolean;
    move_spam_needed: number;
    move_ham_needed: number;
};
