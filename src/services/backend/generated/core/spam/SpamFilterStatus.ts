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
     * Whether the model has seen enough of both classes to classify.
     */
    ready: boolean;
    /**
     * Messages of each class the model needs before it classifies.
     */
    min_per_class: number;
};
