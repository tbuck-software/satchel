import { useConfirm } from '@/composables/useConfirm';
import { useAccountsStore } from '@/stores/accounts';
import { useTranslationStore } from '@/stores/translation';
import { t } from '@/i18n/runtime';

/**
 * Asks before the first message of an account goes to the translation
 * engine, and records the consent on yes (ADR 0016). Its own chunk: the
 * question's copy loads when it is asked, not with every reader.
 *
 * Only a Pro key gets this far; a free key is refused before any question.
 */
export async function askTranslationConsent(accountId: string): Promise<boolean> {
  const account = useAccountsStore().accounts.find((entry) => entry.id === accountId);
  const name = account?.email || account?.displayName || accountId;
  const allowed = await useConfirm().ask({
    title: t('translation.consent.title'),
    body: t('translation.consent.body', { account: name, plan: t('settings.translation.planPro') }),
    detail: `${t('settings.translation.termsPro')} ${t('translation.consent.detail')}`,
    confirmLabel: t('translation.consent.confirm'),
  });
  if (!allowed) return false;
  try {
    await useTranslationStore().setConsent(accountId, true);
    return true;
  } catch {
    return false;
  }
}
