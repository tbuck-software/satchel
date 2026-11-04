import { defineStore } from 'pinia';
import { computed, ref, watch } from 'vue';
import {
  backendCommandsAvailable,
  backendListCertificateExceptions,
  backendRevokeCertificateException,
  backendTrustExpiredCertificate,
  type BackendCertificateException,
} from '@/services/backend';
import { certificateNoticePersistence } from '@/persistence/certificateNotices';
import { useConnectionStore } from '@/stores/connection';
import { useUiStore } from '@/stores/ui';
import {
  accountCertificateRejection,
  rejectionSignature,
  type CertificateRejection,
} from '@/utils/certificateRejection';

export interface CertificateProblem {
  accountId: string;
  rejection: CertificateRejection;
  /** Hidden from the banner; the account keeps its marker. */
  dismissed: boolean;
}

/**
 * Rejected server certificates per account, and the exceptions granted for them.
 *
 * The live failures of the last sync run are the source: once an account syncs
 * again its problem is gone, whatever an older stored error says. "Hide" is
 * remembered per certificate, so the same expired certificate does not come
 * back on the next run or the next launch, and a different one does.
 *
 * Trusting a certificate hides its notice only until the next run reports:
 * success clears it anyway, and when the exception later runs out or is
 * ended, the same certificate has to be able to show up again.
 */
export const useCertificateTrustStore = defineStore('certificateTrust', () => {
  const connection = useConnectionStore();
  const exceptions = ref<BackendCertificateException[]>([]);
  const dismissed = ref<string[]>(certificateNoticePersistence.dismissed.read());
  /** Notices waiting for the run that follows a granted exception. */
  const awaitingRun = ref<string[]>([]);
  /** The account whose details are open, or null. */
  const detailsAccountId = ref<string | null>(null);

  function noticeKey(accountId: string, rejection: CertificateRejection): string {
    return `${accountId}|${rejectionSignature(rejection)}`;
  }

  const problems = computed<CertificateProblem[]>(() => {
    const byAccount = new Map<string, CertificateProblem>();
    for (const failure of connection.syncFailures) {
      const found = accountCertificateRejection(failure);
      if (!found) continue;
      const { accountId, rejection } = found;
      const key = noticeKey(accountId, rejection);
      byAccount.set(accountId, {
        accountId,
        rejection,
        dismissed: dismissed.value.includes(key) || awaitingRun.value.includes(key),
      });
    }
    return [...byAccount.values()];
  });

  /** Problems the banner still has to show. */
  const visibleProblems = computed(() => problems.value.filter((problem) => !problem.dismissed));

  /** Whether the drawer handle should carry a dot: some account needs a look. */
  const needsAttention = computed(() => problems.value.length > 0);

  function problemFor(accountId: string): CertificateProblem | null {
    return problems.value.find((problem) => problem.accountId === accountId) ?? null;
  }

  function exceptionFor(accountId: string): BackendCertificateException | null {
    return exceptions.value.find((exception) => exception.account_id === accountId) ?? null;
  }

  function dismiss(accountId: string) {
    const problem = problemFor(accountId);
    if (!problem) return;
    const key = noticeKey(accountId, problem.rejection);
    if (dismissed.value.includes(key)) return;
    dismissed.value = [...dismissed.value, key];
    certificateNoticePersistence.dismissed.write(dismissed.value);
  }

  /** Drops every remembered "Hide" of an account, so its next notice shows. */
  function forget(accountId: string) {
    const kept = dismissed.value.filter((key) => !key.startsWith(`${accountId}|`));
    if (kept.length === dismissed.value.length) return;
    dismissed.value = kept;
    certificateNoticePersistence.dismissed.write(kept);
  }

  async function load() {
    if (!backendCommandsAvailable()) return;
    try {
      exceptions.value = await backendListCertificateExceptions();
    } catch {
      // Not worth a banner of its own: the exception list only decorates the
      // accounts, and the next sync run asks again.
    }
  }

  /**
   * Accepts the certificate the account's server presented, for seven days.
   * Rejects with the backend's keyed error when the certificate changed in
   * the meantime or is not eligible.
   */
  async function trust(accountId: string): Promise<BackendCertificateException> {
    const problem = problemFor(accountId);
    if (!problem) throw new Error('backend.errors.tlsExceptionNoRejection');
    const exception = await backendTrustExpiredCertificate({
      account_id: accountId,
      host: problem.rejection.host,
      fingerprint: problem.rejection.fingerprint,
    });
    forget(accountId);
    awaitingRun.value = [...awaitingRun.value, noticeKey(accountId, problem.rejection)];
    exceptions.value = [
      ...exceptions.value.filter((entry) => !(entry.account_id === accountId && entry.host === exception.host)),
      exception,
    ];
    connection.requestRetry();
    return exception;
  }

  async function revoke(accountId: string, host: string) {
    await backendRevokeCertificateException(accountId, host);
    forget(accountId);
    exceptions.value = exceptions.value.filter((entry) => !(entry.account_id === accountId && entry.host === host));
    connection.requestRetry();
  }

  function openDetails(accountId: string) {
    detailsAccountId.value = accountId;
  }

  function closeDetails() {
    detailsAccountId.value = null;
  }

  // A finished run may have ended an exception (the server is valid again)
  // or let one expire; the backend settles both, this only reads the result.
  watch(() => connection.lastSyncAt, () => { void load(); });
  // The store replaces the list on every report, so this fires once per run.
  watch(() => connection.syncFailures, () => { awaitingRun.value = []; }, { flush: 'sync' });
  void load();

  const ui = useUiStore();
  watch(needsAttention, (value) => { ui.drawerAttention = value; }, { immediate: true });

  return {
    exceptions,
    problems,
    visibleProblems,
    needsAttention,
    detailsAccountId,
    problemFor,
    exceptionFor,
    dismiss,
    load,
    trust,
    revoke,
    openDetails,
    closeDetails,
  };
});
