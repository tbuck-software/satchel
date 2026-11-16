import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useCertificateTrustStore } from '@/stores/certificateTrust';
import { useConnectionStore } from '@/stores/connection';
import { useOutboxStore } from '@/stores/outbox';
import { useSyncProgress } from './useSyncProgress';
import { accountCertificateRejection } from '@/utils/certificateRejection';

/**
 * What the shell has to say about sync and accounts, as one short phrase.
 *
 * In order of weight: a sign-in that has to happen, a failure the backend
 * reported, a rejected certificate, queue items that need a review, then the
 * two plain states, offline and syncing. Only the first is named; the others
 * that also need action are counted, and the status sheet lists them all.
 * Idle says nothing: a quiet mailbox needs no caption.
 */
export type ShellStatusKind = 'signIn' | 'error' | 'certificate' | 'review' | 'offline' | 'syncing';
export type ShellStatusTone = 'danger' | 'warning' | 'neutral';

const ACTIONABLE: readonly ShellStatusKind[] = ['signIn', 'error', 'certificate', 'review'];

export function useShellStatus() {
  const { t } = useI18n();
  const accounts = useAccountsStore();
  const certificates = useCertificateTrustStore();
  const connection = useConnectionStore();
  const outbox = useOutboxStore();
  const progress = useSyncProgress();

  /** The same split as ConnectionBanner: certificate-only sync failures get the warning, not the error. */
  const onlyCertificates = computed(() => connection.backendErrorSource === 'sync'
    && connection.syncFailures.length > 0
    && connection.syncFailures.every((failure) => accountCertificateRejection(failure)));

  const kinds = computed<ShellStatusKind[]>(() => {
    const list: ShellStatusKind[] = [];
    if (accounts.credentialLock || accounts.accountsWithAuthError.length > 0) list.push('signIn');
    if (connection.backendError && !onlyCertificates.value) list.push('error');
    if (onlyCertificates.value && certificates.visibleProblems.length > 0) list.push('certificate');
    if (outbox.attentionCount > 0) list.push('review');
    if (connection.syncStatus === 'offline') list.push('offline');
    else if (connection.syncStatus === 'syncing' || progress.history.value) list.push('syncing');
    return list;
  });

  const kind = computed<ShellStatusKind | null>(() => kinds.value[0] ?? null);

  /** Issues besides the named one that also want action. */
  const more = computed(() => kinds.value.slice(1).filter((entry) => ACTIONABLE.includes(entry)).length);

  /** Only these open the sheet; a review alone goes straight to the queue. */
  const needsSheet = computed(() => kinds.value.some((entry) => entry === 'signIn' || entry === 'error' || entry === 'certificate'));

  const tone = computed<ShellStatusTone>(() => {
    switch (kind.value) {
      case 'signIn':
      case 'error':
        return 'danger';
      case 'certificate':
      case 'review':
      case 'offline':
        return 'warning';
      default:
        return 'neutral';
    }
  });

  const label = computed(() => {
    switch (kind.value) {
      case 'signIn': return t('app.status.signIn');
      case 'error': return connection.backendErrorSource === 'sync' ? t('app.status.syncFailed') : t('app.status.error');
      case 'certificate': return t('app.status.certificate');
      case 'review': return t('app.status.review', { count: outbox.attentionCount }, outbox.attentionCount);
      case 'offline': return t('app.status.offline');
      case 'syncing': return progress.history.value
        ? (progress.percent.value ? t('app.status.importing', { percent: progress.percent.value }) : progress.label.value)
        : t('app.status.syncing');
      default: return '';
    }
  });

  return { kind, kinds, more, needsSheet, tone, label, progress };
}
