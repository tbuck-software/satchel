import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { usePgpStore } from './pgp';

describe('PGP recipient matching', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('matches Rust ASCII lowercase semantics', () => {
    const store = usePgpStore();
    store.keys = [{
      id: 'alice',
      label: 'Alice',
      key_type: 'public',
      fingerprint: '1234',
      user_ids: ['Alice <ALICE@EXAMPLE.COM>', 'Älice <ÄLICE@EXAMPLE.COM>'],
      has_passphrase: false,
      is_unlocked: false,
      is_expired: false,
      can_encrypt: true,
      created_at: '2026-08-29T00:00:00Z',
      updated_at: '2026-08-29T00:00:00Z',
    }];

    expect(store.hasKeyFor('alice@example.com')).toBe(true);
    expect(store.hasKeyFor('älice@example.com')).toBe(false);
  });

  it('does not offer expired or signing-only keys for encryption', () => {
    const store = usePgpStore();
    store.keys = [
      {
        id: 'expired', label: 'Expired', key_type: 'public', fingerprint: '1',
        user_ids: ['Alice <alice@example.com>'], has_passphrase: false, is_unlocked: false,
        is_expired: true, can_encrypt: true, created_at: '', updated_at: '',
      },
      {
        id: 'signing', label: 'Signing', key_type: 'public', fingerprint: '2',
        user_ids: ['Bob <bob@example.com>'], has_passphrase: false, is_unlocked: false,
        is_expired: false, can_encrypt: false, created_at: '', updated_at: '',
      },
    ];

    expect(store.hasKeyFor('alice@example.com')).toBe(false);
    expect(store.hasKeyFor('bob@example.com')).toBe(false);
  });
});
