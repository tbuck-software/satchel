import type { Message } from '@/types/mail';

export interface ThreadMessageVisibilityOptions {
  expanded: boolean;
  collapseToLatest: boolean;
  isOwnMessage: (message: Message) => boolean;
}

export interface ThreadMessageVisibility {
  visibleMessages: Message[];
  draftMessages: Message[];
  hiddenMessageCount: number;
}

/**
 * Drafts belong to a conversation, but not to its chronology: even an old
 * reply draft must stay visible below the newest received or sent message.
 */
export function threadMessageVisibility(
  messages: readonly Message[],
  options: ThreadMessageVisibilityOptions,
): ThreadMessageVisibility {
  const conversationMessages = messages.filter((message) => !message.isDraft);
  const draftMessages = messages.filter((message) => message.isDraft);

  if (options.expanded) {
    return {
      visibleMessages: conversationMessages,
      draftMessages,
      hiddenMessageCount: 0,
    };
  }

  const latest = conversationMessages.at(-1);
  const visibleCount = options.collapseToLatest
    ? 1
    : latest && options.isOwnMessage(latest) && conversationMessages.length > 1
      ? 2
      : 1;
  const visibleMessages = conversationMessages.slice(-visibleCount);

  return {
    visibleMessages,
    draftMessages,
    hiddenMessageCount: Math.max(0, conversationMessages.length - visibleMessages.length),
  };
}
