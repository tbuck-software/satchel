import { getCurrentScope, onScopeDispose, ref } from 'vue';
import { t } from '@/i18n/runtime';
import {
  backendUnlockPgpKeys,
  backendUnlockSmimeCerts,
} from '@/services/backend';
import { backendErrorKey, translateBackendError } from '@/utils/backendErrors';
import { useMessagesStore } from '@/stores/messages';

/** Which material the passphrase dialog is currently asking about. */
export type SecurityScheme = 'pgp' | 'smime';

/**
 * The backend already knows the difference between "there is no key for this"
 * and "the key is locked". Asking for a passphrase in the first case wastes the
 * user's time on a question that cannot have a right answer.
 */
const PGP_NO_KEY_ERRORS = new Set([
  'backend.errors.pgpNoPrivateKeys',
  'backend.errors.pgpKeyNotFound',
  'backend.errors.pgpPrivateKeyRequired',
]);

const PGP_WRONG_PASSPHRASE = 'backend.errors.pgpPassphraseMismatch';

/**
 * Unlocking message security material is the same interaction for PGP and
 * S/MIME: try the session keys first, ask once when they are protected, then
 * reload the thread so the renderer sees the newly decrypted content.
 *
 * The blind first attempt stays — an unlocked keyring owes nobody a dialog.
 * What changed is everything after it: the question is a real dialog rather
 * than `window.prompt`, and "no key" no longer reads as "unlock failed".
 */
export function useMessageSecurityActions() {
  const messagesStore = useMessagesStore();
  const pgpBusy = ref(false);
  const pgpStatus = ref('');
  const smimeBusy = ref(false);
  const smimeStatus = ref('');
  /** Non-null exactly while the dialog is up. */
  const passphraseRequest = ref<SecurityScheme | null>(null);

  let pendingPassphrase: ((value: string | null) => void) | null = null;

  function askForPassphrase(scheme: SecurityScheme): Promise<string | null> {
    passphraseRequest.value = scheme;
    return new Promise<string | null>((resolve) => {
      pendingPassphrase = resolve;
    });
  }

  function settlePassphrase(value: string | null) {
    passphraseRequest.value = null;
    const resolve = pendingPassphrase;
    pendingPassphrase = null;
    resolve?.(value);
  }

  function submitPassphrase(value: string) {
    settlePassphrase(value);
  }

  function cancelPassphrase() {
    settlePassphrase(null);
  }

  // A message that scrolls out of the thread while its dialog is open must not
  // leave the unlock hanging on a promise nobody can resolve any more.
  if (getCurrentScope()) onScopeDispose(() => settlePassphrase(null));

  async function unlockPgp(threadId: string, messageId?: string): Promise<void> {
    if (pgpBusy.value) return;

    pgpBusy.value = true;
    pgpStatus.value = '';
    try {
      try {
        await backendUnlockPgpKeys();
      } catch (error) {
        if (PGP_NO_KEY_ERRORS.has(backendErrorKey(error) ?? '')) {
          pgpStatus.value = t('mail.pgp.noKeyDescription');
          return;
        }
        const passphrase = await askForPassphrase('pgp');
        if (passphrase === null) return;
        try {
          await backendUnlockPgpKeys({ passphrase: passphrase.trim() || undefined });
        } catch (retryError) {
          pgpStatus.value = backendErrorKey(retryError) === PGP_WRONG_PASSPHRASE
            ? t('mail.pgp.passphraseWrong')
            : t('mail.pgp.unlockFailed');
          return;
        }
      }

      await messagesStore.reloadPgpMessages(threadId);
      const message = messagesStore.getByThread(threadId).find((item) => item.id === messageId);
      if (messageId && (!message || message.bodyLoadError || message.bodyText.includes('-----BEGIN PGP MESSAGE-----'))) {
        pgpStatus.value = translateBackendError(message?.pgp?.errorDetail) ?? t('mail.pgp.decryptFailed');
        return;
      }
      pgpStatus.value = t('mail.pgp.unlocked');
    } catch {
      pgpStatus.value = t('mail.pgp.unlockFailed');
    } finally {
      pgpBusy.value = false;
    }
  }

  async function unlockSmime(threadId: string): Promise<void> {
    if (smimeBusy.value) return;

    smimeBusy.value = true;
    smimeStatus.value = '';
    try {
      try {
        await backendUnlockSmimeCerts();
      } catch {
        // `unlock_smime_certificates` still answers with English prose instead
        // of a `backend.errors.*` key, so the two failures cannot be told
        // apart here the way the PGP ones can.
        const passphrase = await askForPassphrase('smime');
        if (passphrase === null) return;
        await backendUnlockSmimeCerts({ passphrase: passphrase.trim() || undefined });
      }

      await messagesStore.loadByThread(threadId);
      smimeStatus.value = t('mail.smime.unlocked');
    } catch {
      smimeStatus.value = t('mail.smime.unlockFailed');
    } finally {
      smimeBusy.value = false;
    }
  }

  function reset() {
    settlePassphrase(null);
    pgpBusy.value = false;
    pgpStatus.value = '';
    smimeBusy.value = false;
    smimeStatus.value = '';
  }

  return {
    pgpBusy,
    pgpStatus,
    smimeBusy,
    smimeStatus,
    passphraseRequest,
    submitPassphrase,
    cancelPassphrase,
    unlockPgp,
    unlockSmime,
    reset,
  };
}
