import { nextTick, ref, type Ref } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import type { BackendPgpKey, BackendSmimeCert } from '@/services/backend';
import { usePgpStore } from '@/stores/pgp';
import { useSmimeStore } from '@/stores/smime';
import { useComposerCrypto, type ComposerCryptoTranslator } from './useComposerCrypto';

const sender = 'sender@example.com';
const recipient = 'recipient@example.com';

function translate(key: string, params?: Record<string, string | number>): string {
  return params?.recipients ? `${key}:${params.recipients}` : key;
}

function certificate(overrides: Partial<BackendSmimeCert> = {}): BackendSmimeCert {
  return {
    id: 'cert-1',
    label: 'Test certificate',
    cert_type: 'public',
    fingerprint: 'fingerprint',
    subject_email: recipient,
    subject_cn: null,
    issuer_cn: null,
    not_before: null,
    not_after: null,
    has_passphrase: false,
    is_unlocked: true,
    created_at: '',
    updated_at: '',
    ...overrides,
  };
}

function pgpKey(overrides: Partial<BackendPgpKey> = {}): BackendPgpKey {
  return {
    id: 'key-1',
    label: 'Test key',
    key_type: 'public',
    fingerprint: 'fingerprint',
    user_ids: [`Recipient <${recipient}>`],
    has_passphrase: false,
    is_unlocked: false,
    is_expired: false,
    can_encrypt: true,
    created_at: '',
    updated_at: '',
    ...overrides,
  };
}

interface Harness {
  to: Ref<string[]>;
  cc: Ref<string[]>;
  bcc: Ref<string[]>;
  fromAddress: Ref<string>;
  policy: ReturnType<typeof useComposerCrypto>;
}

function createHarness(): Harness {
  const to = ref<string[]>([]);
  const cc = ref<string[]>([]);
  const bcc = ref<string[]>([]);
  const fromAddress = ref(sender);
  return {
    to,
    cc,
    bcc,
    fromAddress,
    policy: useComposerCrypto({ to, cc, bcc, fromAddress, t: translate as ComposerCryptoTranslator }),
  };
}

describe('useComposerCrypto', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('keeps the selected security method mutually exclusive and resets its controls', async () => {
    const smime = useSmimeStore();
    const pgp = usePgpStore();
    smime.certs = [
      certificate({ id: 'sender-cert', cert_type: 'private', subject_email: sender }),
      certificate(),
    ];
    pgp.engine = {
      available: true,
      engine: 'native',
      can_decrypt: true,
      can_sign: true,
      can_encrypt: true,
    };
    pgp.keys = [
      pgpKey({ key_type: 'private', is_unlocked: true, user_ids: [`Sender <${sender}>`] }),
      pgpKey(),
    ];

    const { policy, to, fromAddress } = createHarness();
    to.value = [recipient];
    fromAddress.value = sender;

    policy.cryptoMethod.value = 'smime';
    await nextTick();
    policy.cryptoSign.value = true;
    policy.cryptoEncrypt.value = true;
    expect(policy.smimeSignCertId.value).toBe('sender-cert');
    expect(policy.smimeEncrypt.value).toBe(true);

    policy.cryptoMethod.value = 'pgp';
    await nextTick();
    expect(policy.smimeSignCertId.value).toBeNull();
    expect(policy.smimeEncrypt.value).toBe(false);
    expect(policy.pgpSignKeyId.value).toBeNull();
    expect(policy.pgpEncrypt.value).toBe(false);

    policy.cryptoSign.value = true;
    policy.cryptoEncrypt.value = true;
    expect(policy.pgpSignKeyId.value).toBe('key-1');
    expect(policy.pgpEncrypt.value).toBe(true);

    policy.cryptoMethod.value = 'none';
    await nextTick();
    expect(policy.cryptoSign.value).toBe(false);
    expect(policy.cryptoEncrypt.value).toBe(false);
    expect(policy.pgpSignKeyId.value).toBeNull();
    expect(policy.pgpEncrypt.value).toBe(false);
  });

  it('gates S/MIME controls on sender certificate and every recipient certificate', async () => {
    const smime = useSmimeStore();
    const { policy, to } = createHarness();
    policy.cryptoMethod.value = 'smime';
    await nextTick();

    expect(policy.canSignSelected.value).toBe(false);
    expect(policy.canEncryptSelected.value).toBe(false);
    expect(policy.cryptoHint.value).toContain('compose.crypto.noSmimeCertificate');
    expect(policy.cryptoHint.value).toContain('compose.crypto.addRecipientsFirst');

    smime.certs = [certificate({ id: 'sender-cert', cert_type: 'private', subject_email: sender })];
    to.value = [recipient];
    await nextTick();
    expect(policy.canSignSelected.value).toBe(true);
    expect(policy.canEncryptSelected.value).toBe(false);
    expect(policy.cryptoHint.value).toContain('compose.crypto.missingSmimeCertificates:recipient@example.com');
    policy.cryptoEncrypt.value = true;
    expect(policy.cryptoEncrypt.value).toBe(false);

    smime.certs = [...smime.certs, certificate()];
    await nextTick();
    expect(policy.canEncryptSelected.value).toBe(true);
    policy.cryptoSign.value = true;
    policy.cryptoEncrypt.value = true;
    expect(policy.smimeSignCertId.value).toBe('sender-cert');
    expect(policy.smimeEncrypt.value).toBe(true);

    to.value = [];
    await nextTick();
    expect(policy.canEncryptSelected.value).toBe(false);
    expect(policy.smimeEncrypt.value).toBe(false);
  });

  it('gates PGP controls on an available engine, unlocked signing key, and recipient keys', async () => {
    const pgp = usePgpStore();
    const { policy, to } = createHarness();
    pgp.engine = {
      available: true,
      engine: 'native',
      can_decrypt: true,
      can_sign: true,
      can_encrypt: true,
    };
    pgp.keys = [pgpKey({ key_type: 'private', is_unlocked: true, user_ids: [`Sender <${sender}>`] })];
    policy.cryptoMethod.value = 'pgp';
    await nextTick();

    to.value = [recipient];
    await nextTick();
    expect(policy.canSignSelected.value).toBe(true);
    expect(policy.canEncryptSelected.value).toBe(false);
    expect(policy.cryptoHint.value).toContain('compose.crypto.missingPgpKeys:recipient@example.com');

    pgp.keys = [...pgp.keys, pgpKey()];
    await nextTick();
    expect(policy.canEncryptSelected.value).toBe(true);
    policy.cryptoSign.value = true;
    policy.cryptoEncrypt.value = true;
    expect(policy.pgpSignKeyId.value).toBe('key-1');
    expect(policy.pgpEncrypt.value).toBe(true);

    pgp.engine = { ...pgp.engine, available: false };
    await nextTick();
    expect(policy.canEncryptSelected.value).toBe(false);
    expect(policy.cryptoEncrypt.value).toBe(false);
    expect(policy.cryptoOptions.value.find((option) => option.value === 'pgp')?.disabled).toBe(true);
  });

  it('offers disabled PGP with an unavailable engine and clears a vanished signing certificate', async () => {
    const smime = useSmimeStore();
    const { policy, fromAddress } = createHarness();
    smime.certs = [certificate({ id: 'sender-cert', cert_type: 'private', subject_email: sender })];
    policy.cryptoMethod.value = 'smime';
    await nextTick();
    policy.cryptoSign.value = true;
    expect(policy.smimeSignCertId.value).toBe('sender-cert');

    smime.certs = [];
    fromAddress.value = 'other@example.com';
    await nextTick();
    expect(policy.canSignSelected.value).toBe(false);
    expect(policy.cryptoSign.value).toBe(false);
    expect(policy.smimeSignCertId.value).toBeNull();
    expect(policy.cryptoOptions.value.find((option) => option.value === 'pgp')?.disabled).toBe(true);
  });
});
