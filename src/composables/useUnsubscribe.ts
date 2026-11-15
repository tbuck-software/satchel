import { useConfirm } from '@/composables/useConfirm';
import { t } from '@/i18n/runtime';
import { useMessagesStore } from '@/stores/messages';
import type { BackendUnsubscribeResult } from '@/services/backend/messages';

/**
 * Unsubscribe through a message's `List-Unsubscribe` header. A one-click POST
 * goes straight out; a mail is sent only after the user has seen and
 * confirmed its exact recipient, subject and body. `null` means declined.
 */
export function useUnsubscribe() {
  const messages = useMessagesStore();
  const confirm = useConfirm();

  async function unsubscribe(messageId: string): Promise<BackendUnsubscribeResult | null> {
    const result = await messages.unsubscribe(messageId);
    const mail = result.confirm_mail;
    if (!mail) return result;
    const confirmed = await confirm.ask({
      title: t('mail.unsubscribe.confirm.title'),
      body: t('mail.unsubscribe.confirm.body'),
      fields: [
        { label: t('mail.unsubscribe.confirm.to'), value: mail.to },
        { label: t('mail.unsubscribe.confirm.subject'), value: mail.subject },
        { label: t('mail.unsubscribe.confirm.message'), value: mail.body },
      ],
      confirmLabel: t('mail.unsubscribe.confirm.send'),
    });
    if (!confirmed) return null;
    return messages.unsubscribe(messageId, mail);
  }

  return { unsubscribe };
}
