import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore, type NewSenderGroup } from '@/stores/threads';
import { useThreadActions } from '@/composables/useThreadActions';
import { useToast } from '@/composables/useToast';
import { useConfirm } from '@/composables/useConfirm';
import { t } from '@/i18n/runtime';

/**
 * Review new senders and open the next message after each decision.
 *
 * The review list ticks a sender off locally and never shows it again. Doing
 * that before the backend confirmed the block files an unblocked sender as
 * handled — the mail keeps arriving and nothing ever asks about it again. Both
 * entry points (inbox stack and the review view) run through here so the order
 * cannot drift apart a second time.
 */
export function useNewSenderActions() {
  const settings = useSettingsStore();
  const threads = useThreadsStore();
  const threadActions = useThreadActions();
  const toast = useToast();
  const { ask } = useConfirm();

  function selectNextSender() {
    const next = threads.newSenderStack?.senders[0];
    if (next) threads.selectThread(next.topThreadId);
  }

  function acceptSender(sender: NewSenderGroup) {
    settings.acceptSender(sender.senderEmail);
    selectNextSender();
  }

  /**
   * @param confirmMessage Translated question; the two entry points word it
   *   differently because only one of them also moves the mail away.
   * @returns whether the sender was blocked.
   */
  async function blockSender(sender: NewSenderGroup, confirmMessage: string): Promise<boolean> {
    const confirmed = await ask({
      title: t('newSenders.blockTitle'),
      body: confirmMessage,
      confirmLabel: t('newSenders.block'),
      tone: 'danger',
    });
    if (!confirmed) return false;

    try {
      await threadActions.blockSender({
        pattern: sender.senderEmail,
        accountId: sender.accountId,
      });
    } catch {
      toast.error(t('newSenders.blockFailed', { email: sender.senderEmail }));
      return false;
    }

    settings.setDismissedNewSender(sender.senderEmail, sender.lastMessageAt);
    await threads.load();
    selectNextSender();
    return true;
  }

  return { acceptSender, blockSender };
}
