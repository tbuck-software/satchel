import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendListContacts,
  backendSearchContacts,
  backendCommandsAvailable,
  backendSupportsNativeCommands,
  backendSyncLocalContactAvatars,
  backendSyncContactAvatars,
} from '@/services/backend';
import { useContactsStore } from './contacts';

vi.mock('@/services/backend', () => ({
  backendListContacts: vi.fn(),
  backendSearchContacts: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendSyncLocalContactAvatars: vi.fn(),
  backendSyncContactAvatars: vi.fn(),
}));

const searchMock = vi.mocked(backendSearchContacts);
const nativeMock = vi.mocked(backendCommandsAvailable);

describe('contacts store search', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    nativeMock.mockReturnValue(true);
  });

  it('keeps native capability and backend search behind the contacts owner', async () => {
    const result = [{ id: 'contact-1', email: 'alice@example.com' }] as never;
    searchMock.mockResolvedValue(result);

    await expect(useContactsStore().search('alice', 8)).resolves.toBe(result);

    expect(nativeMock).toHaveBeenCalledOnce();
    expect(searchMock).toHaveBeenCalledWith('alice', 8);
  });

  it('returns an empty result without touching the backend in web mode', async () => {
    nativeMock.mockReturnValue(false);

    await expect(useContactsStore().search('alice', 8)).resolves.toEqual([]);

    expect(searchMock).not.toHaveBeenCalled();
  });
});

/**
 * The catch used to set `loaded` on any failure, justified by web and mock
 * mode. On a real backend that turned a failed query into an address book that
 * looks imported and empty.
 */
describe('contacts store loading', () => {
  const listMock = vi.mocked(backendListContacts);

  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    nativeMock.mockReturnValue(true);
  });

  it('keeps the failure and stays unloaded on a native backend', async () => {
    const failure = new Error('database is locked');
    listMock.mockRejectedValue(failure);
    const store = useContactsStore();

    await store.load();

    expect(store.error).toBe(failure);
    expect(store.loaded).toBe(false);
    expect(store.loading).toBe(false);
  });

  it('still falls back silently where there is no contacts backend', async () => {
    nativeMock.mockReturnValue(false);
    listMock.mockRejectedValue(new Error('not implemented'));
    const store = useContactsStore();

    await store.load();

    expect(store.error).toBeNull();
    expect(store.loaded).toBe(true);
  });

  it('clears an earlier failure once the query works', async () => {
    listMock.mockRejectedValueOnce(new Error('database is locked'));
    const store = useContactsStore();
    await store.load();

    listMock.mockResolvedValueOnce([
      { id: 'contact-1', email: 'alice@example.com' },
    ] as never);
    await store.load();

    expect(store.error).toBeNull();
    expect(store.loaded).toBe(true);
    expect(store.contacts).toHaveLength(1);
  });
});

describe('local contact photos', () => {
  it('reloads newly imported local photos even when native window commands are unavailable', async () => {
    setActivePinia(createPinia());
    vi.mocked(backendCommandsAvailable).mockReturnValue(true);
    vi.mocked(backendSupportsNativeCommands).mockReturnValue(false);
    vi.mocked(backendSyncLocalContactAvatars).mockResolvedValue(1);
    vi.mocked(backendListContacts).mockResolvedValue([{ email: 'person@example.test', avatar_url: 'data:image/png;base64,photo' }] as never);
    const store = useContactsStore();
    expect(await store.refreshLocalAvatars()).toBe(1);
    expect(store.findByEmail('person@example.test')?.avatar_url).toBe('data:image/png;base64,photo');
  });
});


describe('third-party avatar consent', () => {
  it('keeps network lookups off unless the import caller explicitly enables them', async () => {
    setActivePinia(createPinia());
    const sync = vi.mocked(backendSyncContactAvatars);
    sync.mockClear();
    sync.mockResolvedValue({ accounts_checked: 0, avatars_updated: 0, fallback_avatars_updated: 0, errors: [] });
    const store = useContactsStore();
    await store.syncAvatars();
    await store.syncAvatars(true);
    await store.syncAvatars(false);
    expect(sync.mock.calls).toEqual([[false], [true], [false]]);
  });
});
