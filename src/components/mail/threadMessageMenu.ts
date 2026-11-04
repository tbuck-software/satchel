import type { ComputedRef, InjectionKey, ShallowRef } from 'vue';
import type { BodyViewState } from './bodyViewState';

/**
 * The always-allow half of a blocked-images notice, as the message around it
 * sees it. On the phone the notice keeps only Show; these choices join the
 * message's actions (ImageGuardPhoneActions.vue reports them here).
 */
export interface ImageGuardChoices {
  senderEmail: string;
  /** Empty when a domain-wide allow is not offered. */
  domain: string;
  allowSender(): void;
  allowDomain(): void;
}

/**
 * Provided by MessageBubble. Without it (the topic feed) the phone notice
 * keeps the choices on its own chevron.
 */
export const IMAGE_GUARD_CHOICES: InjectionKey<ShallowRef<ImageGuardChoices | null>> = Symbol('imageGuardChoices');

export type MessageMenuCommand = 'reply' | 'replyAll' | 'forward' | 'topic' | 'source' | 'toggleCollapsed';

/**
 * What one message offers on the phone: its own ⋯ sheet and, for the newest
 * message, the thread's nav-bar sheet render the same list from this, and the
 * message runs every command itself.
 */
export interface MessageMenu {
  canReply: boolean;
  /** Someone besides the sender would get the reply. */
  canReplyAll: boolean;
  collapsed: boolean;
  bodyView: BodyViewState | null;
  images: ImageGuardChoices | null;
  run(command: MessageMenuCommand): void;
}

/**
 * The thread reader's side (ThreadView.vue): which message hands its actions
 * to the nav bar's ⋯, which messages keep a ⋯ beside their sender, and the
 * menu the nav-bar message reported. Outside a thread every message keeps
 * its own ⋯.
 */
export interface ThreadMessageMenuHost {
  hostId: ComputedRef<string>;
  inlineIds: ComputedRef<ReadonlySet<string>>;
  menu: ShallowRef<MessageMenu | null>;
}

export const THREAD_MESSAGE_MENU: InjectionKey<ThreadMessageMenuHost> = Symbol('threadMessageMenu');

/**
 * The messages that keep a ⋯ beside their sender, given the thread's messages
 * oldest first (drafts aside). A thread of one has its ⋯ in the nav bar only.
 * In a longer thread every message keeps its own, the newest included: its
 * actions are in the nav-bar sheet as well, where a reader of the latest
 * reply looks first.
 */
export function inlineMessageMenuIds(messageIds: readonly string[]): ReadonlySet<string> {
  if (messageIds.length < 2) return new Set();
  return new Set(messageIds);
}
