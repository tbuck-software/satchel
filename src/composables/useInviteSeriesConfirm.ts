import { useI18n } from 'vue-i18n';
import { useConfirm } from './useConfirm';

export function useInviteSeriesConfirm() {
  const { ask } = useConfirm();
  const { t } = useI18n();
  return () => ask({
    title: t('mail.invite.unboundedSeriesTitle'),
    body: t('mail.invite.unboundedSeriesConfirm'),
    confirmLabel: t('mail.invite.unboundedSeriesAction'),
  });
}
