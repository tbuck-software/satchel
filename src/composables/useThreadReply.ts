import { useComposerStore } from '@/stores/composer';
import { useDraftsStore } from '@/stores/drafts';
import { useMessagesStore } from '@/stores/messages';

/**
 * Reply to a thread, the way every reply entry point does it — the `r` key,
 * the palette, the toolbar menu, the phone's action bar.
 *
 * A thread that already has an unsent draft is answered by that draft: a
 * fresh composer beside it would leave two answers to the same message, and
 * the reader shows the draft as the one way back in. Reply all asks for a
 * different set of recipients, so it starts its own.
 *
 * This lives beside the composer store rather than in it: the store is reached
 * from the message viewer (through the link opener), which must not load the
 * messages or drafts stores.
 */
export function useThreadReply() {
  /**
   * The newest unsent draft the open thread shows, else the newest the drafts
   * list knows for it. A scheduled draft is on its way out, and opening it
   * would cancel the send.
   */
  function draftFor(threadId: string): string | null {
    // Resolved on use, not on setup: most callers never reply.
    const messages = useMessagesStore();
    const drafts = useDraftsStore();
    const threadMessages = messages.getByThread(threadId);
    // Either loaded source can already know that a cached draft is unavailable.
    const unavailable = new Set([
      ...threadMessages.filter(message => message.contentError).map(message => message.id),
      ...drafts.items.filter(draft => draft.content_error).map(draft => draft.id),
    ]);
    const scheduled = new Set(drafts.items.filter((draft) => draft.scheduled_for).map((draft) => draft.id));
    const shown = threadMessages
      .filter((message) => message.isDraft && !unavailable.has(message.id) && !scheduled.has(message.id))
      .at(-1);
    if (shown) return shown.id;
    return drafts.sorted.find((draft) => draft.thread_id === threadId && !draft.scheduled_for && !unavailable.has(draft.id))?.id ?? null;
  }

  function reply(threadId: string, replyAll = false) {
    const composer = useComposerStore();
    const draftId = replyAll ? null : draftFor(threadId);
    if (draftId) return composer.openDraft(draftId);
    return composer.openReply(threadId, replyAll);
  }

  return { reply, draftFor };
}
