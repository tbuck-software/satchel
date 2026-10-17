// @vitest-environment jsdom
import { nextTick } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendCreateFolder,
  backendDeleteFolder,
  backendListMailboxes,
  backendRenameFolder,
  type BackendMailbox,
} from '@/services/backend';
import { useMailboxesStore } from './mailboxes';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendListMailboxes: vi.fn(),
  backendCreateFolder: vi.fn(),
  backendRenameFolder: vi.fn(),
  backendDeleteFolder: vi.fn(),
}));

vi.mock('@/stores/connection', () => ({
  useConnectionStore: () => connectionMock,
}));

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
    clear: () => values.clear(),
  };
}

const listMock = vi.mocked(backendListMailboxes);
const createFolderMock = vi.mocked(backendCreateFolder);
const renameFolderMock = vi.mocked(backendRenameFolder);
const deleteFolderMock = vi.mocked(backendDeleteFolder);

const connectionMock = {
  reportBackendError: vi.fn(),
  clearBackendError: vi.fn(),
};

function mailbox(overrides: Partial<BackendMailbox> = {}): BackendMailbox {
  return {
    id: 'mb-1',
    account_id: 'acc-1',
    account_email: 'you@example.com',
    name: '[IMAP]/Work/Projects',
    mailbox_type: 'custom',
    unread_count: 2,
    thread_count: 5,
    ...overrides,
  } as BackendMailbox;
}

describe('mailboxes store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.stubGlobal('window', { localStorage: memoryStorage() });
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('reports backend errors only when commands are available', async () => {
    const { backendCommandsAvailable } = await import('@/services/backend');
    const nativeFlag = vi.mocked(backendCommandsAvailable);

    listMock.mockRejectedValueOnce(new Error('no database'));
    const mailboxes = useMailboxesStore();
    await mailboxes.load();
    expect(connectionMock.reportBackendError).toHaveBeenCalledWith('mailboxes', expect.any(Error));

    // A pure-browser session stays silent: there is no backend to blame.
    nativeFlag.mockReturnValue(false);
    listMock.mockRejectedValueOnce(new Error('no database'));
    await mailboxes.load();
    expect(connectionMock.reportBackendError).toHaveBeenCalledTimes(1);
    nativeFlag.mockReturnValue(true);
  });

  it('keeps good mailbox data and ignores stale results', async () => {
    const store = useMailboxesStore();
    listMock.mockResolvedValueOnce([mailbox()]);
    await store.load();
    listMock.mockRejectedValueOnce(new Error('offline'));
    await store.load();
    expect(store.mailboxes).toHaveLength(1);
    let finish!: (value: BackendMailbox[]) => void;
    listMock.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const first = store.load();
    listMock.mockResolvedValueOnce([mailbox({ id: 'new' })]);
    await store.load();
    finish([]);
    await first;
    expect(store.mailboxes.map(item => item.id)).toEqual(['new']);
  });

  it('manages provider folders and reloads the projection after each mutation', async () => {
    listMock.mockResolvedValue([]);
    createFolderMock.mockResolvedValue({
      id: 'folder-new', account_id: 'acc-1', name: 'Projects', full_path: 'Projects', folder_type: 'custom',
    });
    renameFolderMock.mockResolvedValue({
      id: 'folder-new', account_id: 'acc-1', name: 'Archive', full_path: 'Archive', folder_type: 'custom',
    });
    deleteFolderMock.mockResolvedValue(undefined);
    const mailboxes = useMailboxesStore();

    await mailboxes.createFolder({ accountId: 'acc-1', name: 'Projects' });
    await mailboxes.renameFolder({ folderId: 'folder-new', newName: 'Archive' });
    await mailboxes.deleteFolder('folder-new');

    expect(createFolderMock).toHaveBeenCalledWith({ accountId: 'acc-1', name: 'Projects' });
    expect(renameFolderMock).toHaveBeenCalledWith({ folderId: 'folder-new', newName: 'Archive' });
    expect(deleteFolderMock).toHaveBeenCalledWith('folder-new');
    expect(listMock).toHaveBeenCalledTimes(3);
    expect(mailboxes.folderManagementAvailable).toBe(true);
  });

  it('derives browsable folders and hides set-asides and synthetic categories', async () => {
    listMock.mockResolvedValue([
      mailbox(), // [IMAP]/Work/Projects → Work/Projects
      mailbox({ id: 'mb-inbox', name: 'INBOX', mailbox_type: 'inbox' }), // hidden
      mailbox({ id: 'mb-aside', name: 'Aside', mailbox_type: 'aside' }), // hidden
      mailbox({
        id: 'mb-cat',
        name: 'CATEGORY_PROMOTIONS',
        mailbox_type: 'custom',
      }), // hidden pattern
      mailbox({ id: 'mb-plain', name: 'Archive/2026', mailbox_type: 'custom' }),
    ]);
    const mailboxes = useMailboxesStore();
    await mailboxes.load();

    const paths = mailboxes.browsableFolders.map((folder) => folder.path).sort();
    expect(paths).toEqual(['Archive/2026', 'Work/Projects']);
  });

  it('builds one sorted tree per account with synthetic parents and rolled-up counts', async () => {
    listMock.mockResolvedValue([
      mailbox(),
      mailbox({
        id: 'mb-sub',
        name: '[IMAP]/Work/Projects/Satchel',
        unread_count: 3,
        thread_count: 4,
      }),
      mailbox({
        id: 'mb-other',
        account_id: 'acc-2',
        account_email: 'other@example.com',
        name: '[IMAP]/Private',
      }),
    ]);
    const mailboxes = useMailboxesStore();
    await mailboxes.load();

    const trees = mailboxes.accountFolderTrees;
    expect(trees.map((group) => group.accountEmail)).toEqual([
      'other@example.com',
      'you@example.com',
    ]);

    const workTree = trees.find((group) => group.accountId === 'acc-1')?.tree;
    expect(workTree).toHaveLength(1); // single root: Work (synthetic)
    const work = workTree?.[0];
    expect(work?.label).toBe('Work');
    expect(work?.exact).toBe(false);
    expect(work?.folderId).toBeUndefined();
    expect(work?.unreadCount).toBe(0);
    expect(work?.children.map((child) => child.label)).toEqual(['Projects']);

    const projects = work?.children[0];
    expect(projects?.unreadCount).toBe(2); // own counts only, not the child's
    expect(projects?.children.map((child) => child.label)).toEqual(['Satchel']);
  });

  it('persists favorites per account and reflects them in the tree', async () => {
    listMock.mockResolvedValue([mailbox()]);
    const mailboxes = useMailboxesStore();
    await mailboxes.load();

    expect(mailboxes.isFavoriteFolder('acc-1', 'work/projects')).toBe(false);
    mailboxes.toggleFavoriteFolder('acc-1', ' Work / Projects ');
    expect(mailboxes.isFavoriteFolder('acc-1', 'Work/Projects')).toBe(true);
    // The persistence watcher flushes on the next tick.
    await nextTick();
    // Persisted normalized under the account scope…
    const stored = JSON.parse(window.localStorage.getItem('satchel.mailboxes.favoriteFolders') ?? '[]');
    expect(stored).toEqual(['acc-1::Work/Projects']);
    // …and survives a fresh store instance reading the same storage.
    setActivePinia(createPinia());
    const reloaded = useMailboxesStore();
    expect(reloaded.favoriteFolderKeys).toEqual(['acc-1::Work/Projects']);
  });

  it('keeps the watcher safe when favorite persistence fails', async () => {
    const setItem = vi.fn(() => {
      throw new Error('storage unavailable');
    });
    vi.stubGlobal('window', {
      localStorage: {
        getItem: () => null,
        setItem,
        removeItem: () => undefined,
      },
    });
    const mailboxes = useMailboxesStore();

    mailboxes.toggleFavoriteFolder('acc-1', 'Work/Projects');
    await expect(nextTick()).resolves.toBeUndefined();
    expect(setItem).toHaveBeenCalledOnce();
    expect(mailboxes.isFavoriteFolder('acc-1', 'Work/Projects')).toBe(true);
  });

  it('aggregates counters scoped by account and folder path', async () => {
    listMock.mockResolvedValue([
      mailbox(),
      mailbox({
        id: 'mb-sub',
        name: '[IMAP]/Work/Projects/Satchel',
        unread_count: 3,
        thread_count: 4,
      }),
    ]);
    const mailboxes = useMailboxesStore();
    await mailboxes.load();

    expect(mailboxes.countForFolder('Work/Projects')).toBe(2);
    expect(mailboxes.countForFolder('Work/Projects', 'acc-other')).toBe(0);
    expect(mailboxes.threadCountForMailboxType('custom', 'acc-1')).toBe(9);
    expect(mailboxes.countForMailboxType('inbox', null)).toBe(0);
    expect(mailboxes.hasMailboxTypeForAccount('aside', 'acc-1')).toBe(false);
  });
});
