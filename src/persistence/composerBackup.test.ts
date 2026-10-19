import { afterEach, describe, expect, it, vi } from 'vitest';
import { setPersistenceStorageProvider, type StorageLike } from './index';
import {
  BACKUP_HEARTBEAT_MS,
  COMPOSER_BACKUP_STORAGE_KEY,
  MAX_BACKUP_HTML_CHARS,
  composerBackupPersistence,
  type ComposerBackup,
} from './composerBackup';

function memoryStorage(initial: Record<string, string> = {}): StorageLike & {
  values: Map<string, string>;
} {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
}

const NOW = Date.parse('2026-10-12T10:00:00Z');

function backup(overrides: Partial<ComposerBackup> = {}): ComposerBackup {
  return {
    key: 'composer-a',
    draftId: null,
    savedAt: new Date(NOW).toISOString(),
    aliveAt: NOW,
    content: {
      accountId: 'acc-1',
      fromName: null, threadId: null, replyTo: null, sendOptions: {},
      scheduledFor: null, signatureId: null, signatureVariant: 'auto',
      signatureOverrideText: null, signatureOverrideHtml: null,
      fromAddress: 'me@example.com',
      to: ['mara@example.com'],
      cc: [],
      bcc: [],
      subject: 'Quarterly numbers',
      body: 'Ten minutes of typing',
      bodyHtml: '<p>Ten minutes of typing</p>',
      plainText: false,
    },
    ...overrides,
  };
}

describe('composer backup persistence', () => {
  let restoreProvider: (() => void) | undefined;

  afterEach(() => {
    restoreProvider?.();
    restoreProvider = undefined;
    vi.useRealTimers();
  });

  function useStorage(initial: Record<string, string> = {}) {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    const storage = memoryStorage(initial);
    restoreProvider?.();
    restoreProvider = setPersistenceStorageProvider(() => storage);
    return storage;
  }

  it('stores a versioned list and offers it only once its composer has stopped beating', () => {
    const storage = useStorage();
    expect(composerBackupPersistence.write(backup())).toBe(true);
    expect(JSON.parse(storage.values.get(COMPOSER_BACKUP_STORAGE_KEY)!)).toMatchObject({ version: 1 });

    // Its composer is still open: another window must not offer the copy.
    expect(composerBackupPersistence.findOrphan({}, NOW + BACKUP_HEARTBEAT_MS)).toBeNull();

    const later = NOW + BACKUP_HEARTBEAT_MS * 4;
    expect(composerBackupPersistence.findOrphan({}, later)?.content.body).toBe('Ten minutes of typing');
  });

  it('matches a draft-bound copy only to that draft', () => {
    useStorage();
    composerBackupPersistence.write(backup({ key: 'composer-a', draftId: 'draft-1' }));
    const later = NOW + BACKUP_HEARTBEAT_MS * 4;

    expect(composerBackupPersistence.findOrphan({ draftId: 'draft-2' }, later)).toBeNull();
    expect(composerBackupPersistence.findOrphan({ draftId: 'draft-1' }, later)?.key).toBe('composer-a');
  });

  it('keeps a heartbeat copy out of reach and drops it on remove', () => {
    useStorage();
    composerBackupPersistence.write(backup());
    composerBackupPersistence.touch('composer-a', NOW + BACKUP_HEARTBEAT_MS * 4);

    expect(composerBackupPersistence.findOrphan({}, NOW + BACKUP_HEARTBEAT_MS * 5)).toBeNull();

    composerBackupPersistence.remove('composer-a');
    expect(composerBackupPersistence.findOrphan({}, NOW + BACKUP_HEARTBEAT_MS * 50)).toBeNull();
  });

  it('drops oversized HTML but keeps the plain text', () => {
    useStorage();
    const huge = backup({
      content: { ...backup().content, bodyHtml: 'x'.repeat(MAX_BACKUP_HTML_CHARS + 1) },
    });
    composerBackupPersistence.write(huge);

    const stored = composerBackupPersistence.findOrphan({}, NOW + BACKUP_HEARTBEAT_MS * 4);
    expect(stored?.content.bodyHtml).toBeNull();
    expect(stored?.content.body).toBe('Ten minutes of typing');
  });

  it('ignores an unknown version and malformed entries', () => {
    useStorage({
      [COMPOSER_BACKUP_STORAGE_KEY]: JSON.stringify({
        version: 1,
        backups: [{ key: 'broken' }, backup({ key: 'composer-ok', aliveAt: 0 })],
      }),
    });
    expect(composerBackupPersistence.findOrphan({}, NOW)?.key).toBe('composer-ok');

    useStorage({
      [COMPOSER_BACKUP_STORAGE_KEY]: JSON.stringify({ version: 2, backups: [backup({ aliveAt: 0 })] }),
    });
    expect(composerBackupPersistence.findOrphan({}, NOW)).toBeNull();
  });

  it('forgets copies older than two weeks', () => {
    useStorage({
      [COMPOSER_BACKUP_STORAGE_KEY]: JSON.stringify({
        version: 1,
        backups: [backup({ aliveAt: 0, savedAt: '2026-09-01T10:00:00Z' })],
      }),
    });
    expect(composerBackupPersistence.findOrphan({}, NOW)).toBeNull();
  });
});
