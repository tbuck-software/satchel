import { shallowRef } from 'vue';

/**
 * The composer open in this window, as the keyboard sees it: what a drag onto
 * it does, offered as commands ("Attach to draft", "Add to draft").
 * `ComposeView` sets it while mounted.
 *
 * Its own module rather than part of the composer store: the message viewer
 * shares the store's chunk and has no composer to reach.
 */
export interface OpenDraftTarget {
  attach(source: { kind: 'threads' | 'messages'; ids: string[] }): Promise<void>;
  /** False while the draft is a mail merge, which has no recipient fields. Reactive. */
  canAddRecipients(): boolean;
  /** Adds the recipients to To. */
  addRecipients(addresses: string[]): void;
}

export const openDraftTarget = shallowRef<OpenDraftTarget | null>(null);
