import { computed, ref, watch } from 'vue';
import { defineStore } from 'pinia';
import {
  backendCreateFolder,
  backendDeleteFolder,
  backendListMailboxes,
  backendRenameFolder,
  backendCommandsAvailable,
  type BackendMailbox,
} from '@/services/backend';
import { useAccountsStore } from '@/stores/accounts';
import { useConnectionStore } from '@/stores/connection';
import { mailboxesPersistence } from '@/persistence/mailboxes';
const HIDDEN_CUSTOM_FOLDER_PATTERNS = [
  /^CATEGORY_(PERSONAL|PROMOTIONS|SOCIAL|UPDATES|FORUMS)$/i,
  /^(GREEN|ORANGE|RED)_CIRCLE$/i,
  /^YELLOW_STAR$/i,
  /^(IMPORTANT|STARRED|UNREAD|CHAT)$/i,
];

export type MailboxSectionKey =
  | 'inbox'
  | 'sent'
  | 'drafts'
  | 'trash'
  | 'archive'
  | 'snoozed'
  | 'aside'
  | 'spam'
  | 'notifications'
  | 'blocked'
  | 'custom';

export interface FolderEntry {
  accountId: string;
  accountEmail: string;
  path: string;
  label: string;
  pathParts: string[];
  unreadCount: number;
  threadCount: number;
  /** Backend folder row id — required to rename or delete the folder. */
  folderId: string;
  mailboxType: string;
}

export interface FolderTreeNode {
  id: string;
  path: string;
  label: string;
  unreadCount: number;
  threadCount: number;
  exact: boolean;
  favorite: boolean;
  children: FolderTreeNode[];
  /**
   * Only set on nodes that correspond to a real backend folder row. Synthetic
   * parent nodes (created purely to hold children) have none and therefore
   * cannot be renamed or deleted.
   */
  folderId?: string;
  mailboxType?: string;
}

export interface FavoriteFolderGroup {
  accountId: string;
  accountEmail: string;
  tree: FolderTreeNode[];
}

function makeFavoriteKey(accountId: string, path: string): string {
  return `${accountId}::${normalizeFolderPath(path)}`;
}

function normalizeFolderPath(path: string): string {
  return path
    .split('/')
    .map((segment) => segment.trim())
    .filter(Boolean)
    .join('/');
}

function splitFolderPath(path: string): string[] {
  return normalizeFolderPath(path).split('/').filter(Boolean);
}

function isHiddenCustomFolder(name: string): boolean {
  return HIDDEN_CUSTOM_FOLDER_PATTERNS.some((pattern) => pattern.test(name));
}

function deriveBrowsableFolderPath(mailbox: BackendMailbox): string | null {
  const rawName = mailbox.name.trim();
  if (!rawName) {
    return null;
  }

  // The provider-visible set-aside folder backs the built-in workflow entry;
  // showing it again in the custom-folder tree would create two destinations
  // for the same state.
  if (mailbox.mailbox_type === 'aside') {
    return null;
  }

  if (mailbox.mailbox_type === 'inbox' && /^inbox$/i.test(rawName)) {
    return null;
  }

  let path = rawName;

  if (/^\[imap\]\//i.test(path)) {
    path = path.replace(/^\[imap\]\//i, '');
  }

  if (/^inbox[./]/i.test(path)) {
    path = path.replace(/^inbox[./]/i, '');
    path = path.replace(/\./g, '/');
  }

  if (mailbox.mailbox_type === 'custom' && isHiddenCustomFolder(path)) {
    return null;
  }

  const normalized = normalizeFolderPath(path);
  if (!normalized) {
    return null;
  }

  const lower = normalized.toLowerCase();
  if (lower === 'inbox') {
    return null;
  }

  return normalized;
}

function sortTreeNodes(nodes: FolderTreeNode[]): FolderTreeNode[] {
  return [...nodes]
    .sort((left, right) => left.label.localeCompare(right.label, undefined, { sensitivity: 'base' }))
    .map((node) => ({
      ...node,
      children: sortTreeNodes(node.children),
    }));
}

function buildFolderTree(entries: FolderEntry[], favoritePaths?: Set<string>): FolderTreeNode[] {
  const nodes = new Map<string, FolderTreeNode>();
  const roots = new Set<string>();

  const ensureNode = (path: string): FolderTreeNode => {
    const normalized = normalizeFolderPath(path);
    const existing = nodes.get(normalized);
    if (existing) {
      return existing;
    }

    const parts = splitFolderPath(normalized);
    const parentPath = parts.slice(0, -1).join('/');
    const node: FolderTreeNode = {
      id: `folder-node:${normalized}`,
      path: normalized,
      label: parts[parts.length - 1] ?? normalized,
      unreadCount: 0,
      threadCount: 0,
      exact: false,
      favorite: favoritePaths?.has(normalized) ?? false,
      children: [],
    };

    nodes.set(normalized, node);

    if (parentPath) {
      const parent = ensureNode(parentPath);
      if (!parent.children.some((child) => child.path === normalized)) {
        parent.children.push(node);
      }
    } else {
      roots.add(normalized);
    }

    return node;
  };

  for (const entry of entries) {
    const node = ensureNode(entry.path);
    node.exact = true;
    node.unreadCount += entry.unreadCount;
    node.threadCount += entry.threadCount;
    node.folderId = entry.folderId;
    node.mailboxType = entry.mailboxType;
  }

  if (favoritePaths) {
    for (const path of favoritePaths) {
      const normalized = normalizeFolderPath(path);
      if (!normalized) {
        continue;
      }
      const parts = splitFolderPath(normalized);
      for (let index = 0; index < parts.length; index += 1) {
        const ancestor = parts.slice(0, index + 1).join('/');
        const node = ensureNode(ancestor);
        node.favorite = node.favorite || favoritePaths.has(ancestor);
      }
    }
  }

  return sortTreeNodes(
    [...roots]
      .map((rootPath) => nodes.get(rootPath))
      .filter((node): node is FolderTreeNode => Boolean(node)),
  );
}

function favoritePathsForAccount(keys: string[], accountId: string): Set<string> {
  return new Set(
    keys
      .filter((key) => key.startsWith(`${accountId}::`))
      .map((key) => key.split('::')[1])
      .filter((path): path is string => Boolean(path)),
  );
}

export const useMailboxesStore = defineStore('mailboxes', () => {
  const connectionStore = useConnectionStore();
  const accounts = useAccountsStore();
  const mailboxes = ref<BackendMailbox[]>([]);
  const favoriteFolderKeys = ref<string[]>(mailboxesPersistence.favoriteFolderKeys.read());

  const browsableFolders = computed<FolderEntry[]>(() =>
    mailboxes.value
      .map((mailbox) => {
        const path = deriveBrowsableFolderPath(mailbox);
        return {
          accountId: mailbox.account_id,
          accountEmail: mailbox.account_email,
          folderId: mailbox.id,
          mailboxType: mailbox.mailbox_type,
          path: path ?? '',
          label: path ? splitFolderPath(path).slice(-1)[0] ?? path : '',
          pathParts: path ? splitFolderPath(path) : [],
          unreadCount: mailbox.unread_count,
          threadCount: mailbox.thread_count,
        } satisfies FolderEntry;
      })
      .filter((entry) => entry.path.length > 0),
  );

  const visibleCustomFolderPaths = computed(() =>
    Array.from(new Set(browsableFolders.value.map((entry) => entry.path))).sort((left, right) =>
      left.localeCompare(right, undefined, { sensitivity: 'base' }),
    ),
  );

  const favoriteFolderSet = computed(() => new Set(favoriteFolderKeys.value));

  const accountFolderTrees = computed(() => {
    const grouped = new Map<string, { accountId: string; accountEmail: string; tree: FolderTreeNode[] }>();

    for (const entry of browsableFolders.value) {
      const current = grouped.get(entry.accountId);
      if (current) {
        continue;
      }
      grouped.set(entry.accountId, {
        accountId: entry.accountId,
        accountEmail: entry.accountEmail,
        tree: [],
      });
    }

    for (const group of grouped.values()) {
      const accountFavorites = favoritePathsForAccount(favoriteFolderKeys.value, group.accountId);
      group.tree = buildFolderTree(
        browsableFolders.value.filter((entry) => entry.accountId === group.accountId),
        accountFavorites,
      );
    }

    return [...grouped.values()].sort((left, right) =>
      left.accountEmail.localeCompare(right.accountEmail, undefined, { sensitivity: 'base' }),
    );
  });

  const favoriteFolderGroups = computed<FavoriteFolderGroup[]>(() =>
    accountFolderTrees.value
      .map((group) => {
        const accountFavorites = favoritePathsForAccount(favoriteFolderKeys.value, group.accountId);

        return {
          accountId: group.accountId,
          accountEmail: group.accountEmail,
          tree: buildFolderTree(
            browsableFolders.value.filter((entry) => {
              if (entry.accountId !== group.accountId) {
                return false;
              }
              return entry.pathParts.some((_, index) =>
                accountFavorites.has(entry.pathParts.slice(0, index + 1).join('/')),
              );
            }),
            accountFavorites,
          ),
        } satisfies FavoriteFolderGroup;
      })
      .filter((group) => group.tree.length > 0),
  );

  const hasVisibleCustomFolders = computed(() => browsableFolders.value.length > 0);
  const folderManagementAvailable = computed(() => backendCommandsAvailable());

  let loadRequestId = 0;

  async function load() {
    const requestId = ++loadRequestId;
    try {
      const rows = await backendListMailboxes();
      if (requestId !== loadRequestId) return;
      mailboxes.value = rows;
      connectionStore.clearBackendError('mailboxes');
    } catch (error) {
      if (requestId !== loadRequestId) return;
      if (backendCommandsAvailable()) {
        connectionStore.reportBackendError('mailboxes', error);
      }
    }
  }

  /**
   * Move unread badges ahead of the backend: `delta` is +1 or -1 per thread
   * that flipped. Counts are distinct unread threads, and the Inbox leaves out
   * threads that are set aside, done or snoozed — the same rule as the query.
   */
  function adjustUnreadThreads(
    changes: ReadonlyArray<{ accountId: string; mailboxNames: readonly string[]; workflowState: string; delta: number }>,
  ) {
    if (changes.length === 0) return;
    const next = mailboxes.value.map((mailbox) => {
      const name = mailbox.name.trim().toLowerCase();
      let delta = 0;
      for (const change of changes) {
        if (change.accountId !== mailbox.account_id) continue;
        if (!change.mailboxNames.some((candidate) => candidate.trim().toLowerCase() === name)) continue;
        if (
          mailbox.mailbox_type.trim().toLowerCase() === 'inbox'
          && ['aside', 'done', 'snoozed'].includes(change.workflowState)
        ) continue;
        delta += change.delta;
      }
      return delta === 0 ? mailbox : { ...mailbox, unread_count: Math.max(0, mailbox.unread_count + delta) };
    });
    mailboxes.value = next;
  }

  /** Manage provider folders and publish the refreshed folder projection. */
  async function createFolder(input: Parameters<typeof backendCreateFolder>[0]) {
    const folder = await backendCreateFolder(input);
    await load();
    return folder;
  }

  async function renameFolder(input: Parameters<typeof backendRenameFolder>[0]) {
    const folder = await backendRenameFolder(input);
    await load();
    return folder;
  }

  async function deleteFolder(folderId: string) {
    await backendDeleteFolder(folderId);
    await load();
  }

  function countForMailboxType(mailboxType: string, accountId?: string | null): number {
    return mailboxes.value
      .filter((mailbox) => mailbox.mailbox_type === mailboxType)
      .filter((mailbox) => (accountId ? mailbox.account_id === accountId : accounts.isVisible(mailbox.account_id)))
      .reduce((sum, mailbox) => sum + mailbox.unread_count, 0);
  }

  function threadCountForMailboxType(mailboxType: string, accountId?: string | null): number {
    return mailboxes.value
      .filter((mailbox) => mailbox.mailbox_type === mailboxType)
      .filter((mailbox) => (accountId ? mailbox.account_id === accountId : accounts.isVisible(mailbox.account_id)))
      .reduce((sum, mailbox) => sum + mailbox.thread_count, 0);
  }

  function countForFolder(path: string, accountId?: string | null): number {
    const normalized = normalizeFolderPath(path);
    return browsableFolders.value
      .filter((folder) => folder.path === normalized)
      .filter((folder) => (accountId ? folder.accountId === accountId : accounts.isVisible(folder.accountId)))
      .reduce((sum, folder) => sum + folder.unreadCount, 0);
  }

  function hasMailboxTypeForAccount(mailboxType: string, accountId: string): boolean {
    return mailboxes.value.some(
      (mailbox) => mailbox.mailbox_type === mailboxType && mailbox.account_id === accountId,
    );
  }

  function hasMailboxType(mailboxType: string): boolean {
    return mailboxes.value.some((mailbox) => mailbox.mailbox_type === mailboxType);
  }

  function isFavoriteFolder(accountId: string, path: string): boolean {
    return favoriteFolderSet.value.has(makeFavoriteKey(accountId, path));
  }

  function toggleFavoriteFolder(accountId: string, path: string) {
    const normalized = normalizeFolderPath(path);
    if (!normalized) {
      return;
    }

    const key = makeFavoriteKey(accountId, normalized);

    favoriteFolderKeys.value = favoriteFolderSet.value.has(key)
      ? favoriteFolderKeys.value.filter((entry) => entry !== key)
      : [...favoriteFolderKeys.value, key];
  }

  watch(
    favoriteFolderKeys,
    (next) => {
      mailboxesPersistence.favoriteFolderKeys.write(next);
    },
    { deep: true },
  );

  return {
    mailboxes,
    favoriteFolderKeys,
    browsableFolders,
    visibleCustomFolderPaths,
    favoriteFolderGroups,
    accountFolderTrees,
    hasVisibleCustomFolders,
    folderManagementAvailable,
    load,
    adjustUnreadThreads,
    createFolder,
    renameFolder,
    deleteFolder,
    countForMailboxType,
    threadCountForMailboxType,
    countForFolder,
    hasMailboxTypeForAccount,
    hasMailboxType,
    isFavoriteFolder,
    toggleFavoriteFolder,
  };
});
