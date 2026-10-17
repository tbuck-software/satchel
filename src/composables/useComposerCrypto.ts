import { computed, ref, watch, type Ref } from 'vue';
import { usePgpStore } from '@/stores/pgp';
import { useSmimeStore } from '@/stores/smime';

export type ComposerCryptoMethod = 'none' | 'smime' | 'pgp';

export type ComposerCryptoTranslator = (
  key: string,
  params?: Record<string, string | number>,
) => string;

export interface ComposerCryptoOptions {
  to: Ref<string[]>;
  cc: Ref<string[]>;
  bcc: Ref<string[]>;
  fromAddress: Ref<string>;
  t: ComposerCryptoTranslator;
}

/**
 * Owns the message-level S/MIME/PGP policy for the composer.
 *
 * The backend accepts either S/MIME or PGP, never both. This composable keeps
 * that invariant, derives recipient gating from the stores, and exposes only
 * the controls and payload values the composer needs.
 */
export function useComposerCrypto(options: ComposerCryptoOptions) {
  const smimeStore = useSmimeStore();
  const pgpStore = usePgpStore();

  const smimeSignCertId = ref<string | null>(null);
  const smimeEncrypt = ref(false);
  const pgpSignKeyId = ref<string | null>(null);
  const pgpEncrypt = ref(false);

  const allRecipients = computed(() => [
    ...options.to.value,
    ...options.cc.value,
    ...options.bcc.value,
  ]);
  const smimeSigningCert = computed(() =>
    smimeStore.privateCertForEmail(options.fromAddress.value),
  );
  const missingSmimeRecipients = computed(() =>
    allRecipients.value.filter((email) => !smimeStore.certForEmail(email)),
  );
  const canSmimeEncrypt = computed(
    () => allRecipients.value.length > 0 && missingSmimeRecipients.value.length === 0,
  );

  const pgpSigningKey = computed(() => pgpStore.unlockedPrivateKeys[0] ?? null);
  const missingPgpRecipients = computed(() =>
    allRecipients.value.filter((email) => !pgpStore.hasKeyFor(email)),
  );
  const canPgpEncrypt = computed(
    () => pgpStore.available && allRecipients.value.length > 0 && missingPgpRecipients.value.length === 0,
  );

  /** PGP and S/MIME selections are mutually exclusive. */
  const cryptoMethod = ref<ComposerCryptoMethod>('none');

  watch(smimeSigningCert, (cert) => {
    if (smimeSignCertId.value && smimeSignCertId.value !== cert?.id) {
      smimeSignCertId.value = null;
    }
  });

  watch(pgpSigningKey, (key) => {
    if (pgpSignKeyId.value && pgpSignKeyId.value !== key?.id) {
      pgpSignKeyId.value = null;
    }
  });

  watch(canSmimeEncrypt, (canEncrypt) => {
    if (!canEncrypt) smimeEncrypt.value = false;
  });

  watch(canPgpEncrypt, (canEncrypt) => {
    if (!canEncrypt) pgpEncrypt.value = false;
  });

  watch(cryptoMethod, () => {
    smimeSignCertId.value = null;
    smimeEncrypt.value = false;
    pgpSignKeyId.value = null;
    pgpEncrypt.value = false;
  });

  const cryptoOptions = computed(() => [
    { value: 'none' as const, label: options.t('common.none') },
    { value: 'smime' as const, label: 'S/MIME' },
    {
      value: 'pgp' as const,
      label: 'PGP',
      disabled: !pgpStore.available,
      title: pgpStore.available ? undefined : options.t('compose.crypto.pgpUnavailable'),
    },
  ]);

  const canSignSelected = computed(() =>
    cryptoMethod.value === 'smime'
      ? Boolean(smimeSigningCert.value)
      : cryptoMethod.value === 'pgp'
        ? Boolean(pgpSigningKey.value)
        : false,
  );

  const canEncryptSelected = computed(() =>
    cryptoMethod.value === 'smime'
      ? canSmimeEncrypt.value
      : cryptoMethod.value === 'pgp'
        ? canPgpEncrypt.value
        : false,
  );

  const cryptoSign = computed({
    get: () =>
      cryptoMethod.value === 'smime'
        ? Boolean(smimeSignCertId.value)
        : cryptoMethod.value === 'pgp'
          ? Boolean(pgpSignKeyId.value)
          : false,
    set: (value: boolean) => {
      if (cryptoMethod.value === 'smime') {
        smimeSignCertId.value = value && smimeSigningCert.value ? smimeSigningCert.value.id : null;
      } else if (cryptoMethod.value === 'pgp') {
        pgpSignKeyId.value = value && pgpSigningKey.value ? pgpSigningKey.value.id : null;
      }
    },
  });

  const cryptoEncrypt = computed({
    get: () =>
      cryptoMethod.value === 'smime'
        ? smimeEncrypt.value
        : cryptoMethod.value === 'pgp'
          ? pgpEncrypt.value
          : false,
    set: (value: boolean) => {
      if (cryptoMethod.value === 'smime') smimeEncrypt.value = value && canSmimeEncrypt.value;
      else if (cryptoMethod.value === 'pgp') pgpEncrypt.value = value && canPgpEncrypt.value;
    },
  });

  const cryptoHint = computed(() => {
    const parts: string[] = [];
    if (cryptoMethod.value === 'smime') {
      if (!smimeSigningCert.value) {
        parts.push(options.t('compose.crypto.noSmimeCertificate'));
      }
      if (!canSmimeEncrypt.value) {
        parts.push(
          allRecipients.value.length === 0
            ? options.t('compose.crypto.addRecipientsFirst')
            : options.t('compose.crypto.missingSmimeCertificates', {
                recipients: missingSmimeRecipients.value.join(', '),
              }),
        );
      }
    } else if (cryptoMethod.value === 'pgp') {
      if (!pgpSigningKey.value) {
        parts.push(options.t('compose.crypto.noPgpKey'));
      }
      if (!canPgpEncrypt.value) {
        parts.push(
          allRecipients.value.length === 0
            ? options.t('compose.crypto.addRecipientsFirst')
            : options.t('compose.crypto.missingPgpKeys', {
                recipients: missingPgpRecipients.value.join(', '),
              }),
        );
      }
    }
    return parts.join(' ');
  });

  function load(): Promise<unknown[]> {
    return Promise.all([smimeStore.load(), pgpStore.load()]);
  }

  return {
    load,
    smimeSignCertId,
    smimeEncrypt,
    pgpSignKeyId,
    pgpEncrypt,
    cryptoMethod,
    cryptoOptions,
    canSignSelected,
    canEncryptSelected,
    cryptoSign,
    cryptoEncrypt,
    cryptoHint,
  };
}
