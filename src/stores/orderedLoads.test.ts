import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick, reactive } from 'vue';

const backend = vi.hoisted(() => Object.fromEntries([
  'ListTemplates', 'UpsertTemplate', 'DeleteTemplate', 'RenderTemplate',
  'ListSmartFolders', 'UpsertSmartFolder', 'DeleteSmartFolder',
  'ListDrafts', 'SaveDraft', 'DeleteDraft', 'LoadDraft',
  'ListInvites', 'ListCalendarEvents', 'ListCalendarSources', 'RespondToInvite',
  'CreateCalendarEvent', 'DeleteCalendarEvent', 'ImportIcs', 'SyncGoogleCalendar', 'SyncGroupware',
  'ListTopics', 'UpsertTopic', 'DeleteTopic', 'AddTopicItems', 'RemoveTopicItem', 'UndoTopicMemberships',
  'ListTopicMemberships', 'ListTopicFiles', 'AddTopicFile', 'DeleteTopicFile',
  'GetTopicTimeline', 'UpsertTopicNote', 'DeleteTopicNote', 'DownloadTopicFile',
].map((name) => [`backend${name}`, vi.fn()])));
vi.mock('@/services/backend', () => ({ ...backend, backendCommandsAvailable: () => true }));
vi.mock('@/stores/messages', () => ({ useMessagesStore: () => ({ applySavedDraft: vi.fn() }) }));
let scope = reactive({ visibilityKey: 'a' });
vi.mock('@/stores/accounts', () => ({ useAccountsStore: () => scope }));

import { useTemplatesStore } from './templates';
import { useSmartFoldersStore } from './smartFolders';
import { useDraftsStore } from './drafts';
import { useInvitesStore } from './invites';
import { useTopicsStore } from './topics';
import { useThreadActions } from '@/composables/useThreadActions';
import { useToast } from '@/composables/useToast';

const threadState = {};
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => threadState }));
vi.mock('@/stores/mailboxes', () => ({ useMailboxesStore: () => ({}) }));
vi.mock('@/composables/useWorkflow', () => ({ useWorkflow: () => ({}) }));
vi.mock('@/composables/useListKeyboardNavigation', () => ({ focusListThread: vi.fn() }));
vi.mock('@/router/mailViews', () => ({ openView: vi.fn() }));
vi.mock('vue-router', () => ({ useRoute: () => ({}), useRouter: () => ({}) }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const row = { id: 'one', name: 'Saved', body_text: '', query: 'is:unread', updated_at: '', account_id: 'a' };

beforeEach(() => {
  setActivePinia(createPinia());
  vi.resetAllMocks();
  scope = reactive({ visibilityKey: 'a' });
  for (const [name, mock] of Object.entries(backend)) {
    mock.mockResolvedValue(name.includes('List') ? [] : row);
  }
});

for (const [name, useStore, list, save] of [
  ['templates', useTemplatesStore, 'ListTemplates', 'UpsertTemplate'],
  ['smart folders', useSmartFoldersStore, 'ListSmartFolders', 'UpsertSmartFolder'],
  ['drafts', useDraftsStore, 'ListDrafts', 'SaveDraft'],
  ['topics', useTopicsStore, 'ListTopics', 'UpsertTopic'],
] as const) {
  describe(`${name} list ordering`, () => {
    it('rereads a late initial snapshot after a save instead of erasing the saved item', async () => {
      const old = deferred<never[]>();
      backend[`backend${list}`]!.mockReturnValueOnce(old.promise).mockResolvedValue([row]);
      const store = useStore();
      const load = store.load();
      await nextTick();
      if ('save' in store) await store.save({ accountId: 'a', to: [], body: '', subject: '' });
      else await store.upsert({ id: 'one', name: 'Saved', query: 'is:unread', bodyText: '' });
      old.resolve([]);
      await load;
      expect(store.items).toEqual([row]);
      expect(store.loading).toBe(false);
    });

    it('ignores an older failure and its finalizer while a newer request loads', async () => {
      const old = deferred<never[]>();
      const fresh = deferred<never[]>();
      backend[`backend${list}`]!.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
      const store = useStore();
      const first = store.load();
      await nextTick();
      const second = store.load();
      await nextTick();
      old.reject(new Error('old failure'));
      await first;
      expect(store.loading).toBe(true);
      if ('loadFailed' in store) expect(store.loadFailed).toBe(false);
      fresh.resolve([]);
      await second;
      expect(store.loading).toBe(false);
    });

    it('keeps the newer completed response when an old request succeeds later', async () => {
      const old = deferred<unknown[]>();
      backend[`backend${list}`]!.mockReturnValueOnce(old.promise).mockResolvedValue([row]);
      const store = useStore();
      const first = store.load();
      await nextTick();
      await store.load();
      old.resolve([]);
      await first;
      expect(store.items).toEqual([row]);
      expect(store.loading).toBe(false);
    });

    it('does not resurrect a removed item from a late response and permits retry after failed writes', async () => {
      const store = useStore();
      store.items = [row] as never;
      const old = deferred<never[]>();
      backend[`backend${list}`]!.mockReturnValueOnce(old.promise).mockResolvedValue([]);
      const load = store.load();
      await nextTick();
      await store.remove('one');
      old.resolve([row] as never);
      await load;
      expect(store.items).toEqual([]);
      backend[`backend${save}`]!.mockRejectedValueOnce(new Error('write refused'));
      const write = 'save' in store
        ? store.save({ accountId: 'a', to: [], body: '', subject: '' })
        : store.upsert({ id: 'one', name: 'Saved', query: '', bodyText: '' });
      await expect(write).rejects.toThrow('write refused');
      backend[`backend${list}`]!.mockResolvedValue([row]);
      await store.load();
      expect(store.items).toEqual([row]);
    });
  });
}

for (const [useStore, list] of [[useTemplatesStore, 'ListTemplates'], [useSmartFoldersStore, 'ListSmartFolders']] as const) {
  it(`${list} retains data on failed refresh and clears the error on retry`, async () => {
    const store = useStore();
    store.items = [row] as never;
    backend[`backend${list}`]!.mockRejectedValueOnce(new Error('offline'));
    await store.load();
    expect(store.items).toEqual([row]);
    expect(store.loadFailed).toBe(true);
    backend[`backend${list}`]!.mockResolvedValue([]);
    await store.load();
    expect(store.items).toEqual([]);
    expect(store.loadFailed).toBe(false);
  });

  it(`${list} retains a restored item when the pre-restore snapshot arrives late`, async () => {
    const store = useStore();
    const old = deferred<never[]>();
    backend[`backend${list}`]!.mockReturnValueOnce(old.promise).mockResolvedValue([row]);
    const load = store.load();
    await nextTick();
    await store.restore(row as never);
    old.resolve([]);
    await load;
    expect(store.items).toEqual([row]);
  });
}

it('keeps an RSVP and created event after a late initial calendar snapshot', async () => {
  const store = useInvitesStore();
  const invite = { id: 'invite', rsvp_status: 'needs-action' };
  store.items = [invite] as never;
  const old = deferred<unknown[]>();
  backend.backendListInvites!.mockReturnValueOnce(old.promise).mockResolvedValue([{ ...invite, rsvp_status: 'accepted' }]);
  backend.backendCreateCalendarEvent!.mockResolvedValue(row);
  const load = store.load();
  await nextTick();
  await store.respond('invite', 'accepted');
  await store.createEvent({} as never);
  backend.backendListCalendarEvents!.mockResolvedValue([row]);
  old.resolve([invite]);
  await load;
  expect(store.items[0]?.rsvp_status).toBe('accepted');
  expect(store.events).toEqual([row]);
});

it('does not publish topic counts from a previous account visibility scope', async () => {
  const store = useTopicsStore();
  const old = deferred<unknown[]>();
  backend.backendListTopics!.mockReturnValueOnce(old.promise).mockResolvedValue([{ ...row, item_count: 0 }]);
  const first = store.load();
  await nextTick();
  scope.visibilityKey = 'b';
  await nextTick();
  old.resolve([{ ...row, item_count: 99 }]);
  await first;
  await vi.waitFor(() => expect(store.items).toEqual([{ ...row, item_count: 0 }]));
});

it('keeps membership links when a pre-link read arrives late', async () => {
  const store = useTopicsStore();
  const old = deferred<unknown[]>();
  backend.backendListTopicMemberships!.mockReturnValueOnce(old.promise)
    .mockResolvedValue([{ item_id: 'thread', topic_id: 'topic' }]);
  const load = store.loadMembershipsFor('thread', ['thread']);
  await nextTick();
  backend.backendAddTopicItems!.mockResolvedValue([{ item_id: 'thread', revision: 'added' }]);
  await store.addItemsToTopic('topic', 'thread', ['thread']);
  old.resolve([]);
  await load;
  expect(store.topicIdsFor('thread', 'thread')).toEqual(['topic']);
});

it('uses the database receipt when membership lookup fails or the cache is stale', async () => {
  const store = useTopicsStore();
  backend.backendListTopicMemberships!.mockRejectedValue(new Error('offline'));
  await store.loadMembershipsFor('thread', ['existing']);
  backend.backendAddTopicItems!.mockResolvedValue([]);
  await expect(store.addItemsToTopic('topic', 'thread', ['existing'])).resolves.toEqual([]);
  expect(store.topicIdsFor('thread', 'existing')).toEqual(['topic']);
});

it('serializes concurrent writes to the same topic and returns only the actual new link', async () => {
  const store = useTopicsStore();
  const first = deferred<{ item_id: string; revision: string }[]>();
  backend.backendAddTopicItems!.mockReturnValueOnce(first.promise).mockResolvedValue([]);
  const add = store.addItemsToTopic('topic', 'thread', ['thread']);
  const again = store.addItemsToTopic('topic', 'thread', ['thread']);
  expect(backend.backendAddTopicItems).toHaveBeenCalledTimes(1);
  first.resolve([{ item_id: 'thread', revision: 'first' }]);
  expect(await add).toEqual([{ item_id: 'thread', revision: 'first' }]);
  expect(await again).toEqual([]);
  expect(store.topicIdsFor('thread', 'thread')).toEqual(['topic']);
});

it('does not resurrect a deleted event from a late calendar response', async () => {
  const store = useInvitesStore();
  store.events = [row] as never;
  const old = deferred<unknown[]>();
  backend.backendListCalendarEvents!.mockReturnValueOnce(old.promise).mockResolvedValue([]);
  const load = store.load();
  await nextTick();
  await store.deleteEvent('one');
  old.resolve([row]);
  await load;
  expect(store.events).toEqual([]);
});

it('keeps a restored draft when the pre-restore snapshot arrives late', async () => {
  const store = useDraftsStore();
  const old = deferred<unknown[]>();
  backend.backendListDrafts!.mockReturnValueOnce(old.promise).mockResolvedValue([row]);
  const load = store.load();
  await nextTick();
  await store.restore(row as never);
  old.resolve([]);
  await load;
  expect(store.items).toEqual([row]);
});

it('keeps topic files saved after a file list began and rereads after deletion', async () => {
  const store = useTopicsStore();
  const old = deferred<unknown[]>();
  backend.backendListTopicFiles!.mockReturnValueOnce(old.promise).mockResolvedValue([row]);
  const load = store.loadFiles('topic');
  await nextTick();
  await store.addFile({ topicId: 'topic', filename: 'one.txt', contentBase64: 'AA==' });
  old.resolve([]);
  await load;
  expect(store.filesFor('topic')).toEqual([row]);
  const oldAgain = deferred<unknown[]>();
  backend.backendListTopicFiles!.mockReturnValueOnce(oldAgain.promise).mockResolvedValue([]);
  const reload = store.loadFiles('topic');
  await nextTick();
  await store.deleteFile('topic', 'one');
  oldAgain.resolve([row]);
  await reload;
  expect(store.filesFor('topic')).toEqual([]);
});

it('allows a failed membership write to retry without removing a concurrent different topic link', async () => {
  const store = useTopicsStore();
  const failing = deferred<string[]>();
  backend.backendAddTopicItems!.mockReturnValueOnce(failing.promise).mockResolvedValue([{ item_id: 'thread', revision: 'added' }]);
  const add = store.addItemsToTopic('first', 'thread', ['thread']);
  const rejected = expect(add).rejects.toThrow('refused');
  await store.addItemsToTopic('other', 'thread', ['thread']);
  failing.reject(new Error('refused'));
  await rejected;
  expect(store.topicIdsFor('thread', 'thread')).toEqual(['other']);
  await store.addItemsToTopic('first', 'thread', ['thread']);
  expect(store.topicIdsFor('thread', 'thread')).toEqual(['other', 'first']);
});

it('Undo preserves the pre-existing membership after a failed lookup, through the public thread action', async () => {
  const store = useTopicsStore();
  const remote = new Set(['existing']);
  backend.backendListTopicMemberships!.mockRejectedValue(new Error('lookup failed'));
  await store.loadMembershipsFor('thread', ['existing', 'fresh']);
  backend.backendAddTopicItems!.mockImplementation(async ({ itemIds }: { itemIds: string[] }) => {
    const added = itemIds.filter((id) => !remote.has(id));
    added.forEach((id) => remote.add(id));
    return added.map((item_id) => ({ item_id, revision: 'add' }));
  });
  backend.backendUndoTopicMemberships!.mockImplementation(async (_topic, _type, receipts: { item_id: string }[]) => {
    receipts.forEach((receipt) => remote.delete(receipt.item_id));
    return receipts.map((receipt) => receipt.item_id);
  });
  const toast = useToast();
  toast.state.toasts.splice(0);
  await useThreadActions().addToTopic(['existing', 'fresh'], 'topic');
  const undo = toast.state.toasts.at(-1)?.action;
  expect(undo?.label).toBe('Undo');
  await undo!.handler();
  expect(backend.backendUndoTopicMemberships).toHaveBeenCalledExactlyOnceWith('topic', 'thread', [{ item_id: 'fresh', revision: 'add' }], false);
  expect([...remote]).toEqual(['existing']);
  expect(store.topicIdsFor('thread', 'existing')).toEqual(['topic']);
});

it('a stale membership response cannot erase a newer membership request', async () => {
  const store = useTopicsStore();
  const old = deferred<unknown[]>();
  backend.backendListTopicMemberships!.mockReturnValueOnce(old.promise)
    .mockResolvedValue([{ item_id: 'thread', topic_id: 'new' }]);
  const load = store.loadMembershipsFor('thread', ['thread']);
  await nextTick();
  await store.loadMembershipsFor('thread', ['thread']);
  old.resolve([{ item_id: 'thread', topic_id: 'old' }]);
  await load;
  expect(store.topicIdsFor('thread', 'thread')).toEqual(['new']);
});

it('partial receipt updates do not mark other memberships as already loaded', async () => {
  const store = useTopicsStore();
  backend.backendAddTopicItems!.mockResolvedValue([{ item_id: 'thread', revision: 'added' }]);
  await store.addItemsToTopic('new', 'thread', ['thread']);
  expect(store.hasMembershipsFor('thread', 'thread')).toBe(false);
  backend.backendListTopicMemberships!.mockResolvedValue([
    { item_id: 'thread', topic_id: 'new' }, { item_id: 'thread', topic_id: 'existing' },
  ]);
  await store.loadMembershipsFor('thread', ['thread']);
  expect(store.hasMembershipsFor('thread', 'thread')).toBe(true);
  expect(store.topicIdsFor('thread', 'thread')).toEqual(['new', 'existing']);
});

it('an older calendar finalizer cannot stop a newer request loading', async () => {
  const store = useInvitesStore();
  const old = deferred<unknown[]>();
  const fresh = deferred<unknown[]>();
  backend.backendListInvites!.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
  const first = store.load();
  await nextTick();
  const second = store.load();
  await nextTick();
  old.reject(new Error('stale failure'));
  await first;
  expect(store.loading).toBe(true);
  fresh.resolve([row]);
  await second;
  expect(store.items).toEqual([row]);
  expect(store.loading).toBe(false);
});

it('an older draft load cannot clear a save failure, and an explicit retry can clear it', async () => {
  const store = useDraftsStore();
  const old = deferred<unknown[]>();
  backend.backendListDrafts!.mockReturnValueOnce(old.promise).mockResolvedValue([]);
  const load = store.load();
  await nextTick();
  backend.backendSaveDraft!.mockRejectedValueOnce(new Error('disk full'));
  await expect(store.save({ accountId: 'a', to: [], subject: '', body: '' })).rejects.toThrow('disk full');
  old.resolve([]);
  await load;
  expect(store.lastError).toBe('disk full');
  await store.load();
  expect(store.lastError).toBeNull();
});

it('refreshes every affected item when a batch membership read begins during a multi-item write', async () => {
  const store = useTopicsStore();
  const pendingWrite = deferred<{ item_id: string; revision: string }[]>();
  let remote: string[] = [];
  backend.backendAddTopicItems!.mockReturnValueOnce(pendingWrite.promise);
  backend.backendListTopicMemberships!.mockImplementation(async () =>
    remote.map((id) => ({ item_id: id, topic_id: 'topic' })));
  const write = store.addItemsToTopic('topic', 'thread', ['a', 'b', 'c']);
  const read = store.loadMembershipsFor('thread', ['a', 'b', 'c', 'unrelated']);
  await nextTick();
  expect(backend.backendListTopicMemberships).toHaveBeenCalledTimes(1);
  remote = ['a', 'b', 'c'];
  pendingWrite.resolve(remote.map((item_id) => ({ item_id, revision: 'added' })));
  await Promise.all([write, read]);
  for (const id of remote) expect(store.topicIdsFor('thread', id)).toEqual(['topic']);
  expect(store.topicIdsFor('thread', 'unrelated')).toEqual([]);
});

it('does not resurrect a deleted topic from a batch membership read begun during deletion', async () => {
  const store = useTopicsStore();
  let present = true;
  backend.backendListTopicMemberships!.mockImplementation(async () =>
    present ? ['a', 'b'].map((id) => ({ item_id: id, topic_id: 'topic' })) : []);
  await store.loadMembershipsFor('thread', ['a', 'b']);
  const pendingDelete = deferred<void>();
  backend.backendDeleteTopic!.mockReturnValueOnce(pendingDelete.promise);
  const remove = store.remove('topic');
  const read = store.loadMembershipsFor('thread', ['a', 'b', 'unrelated']);
  await nextTick();
  present = false;
  pendingDelete.resolve();
  await Promise.all([remove, read]);
  expect(store.topicIdsFor('thread', 'a')).toEqual([]);
  expect(store.topicIdsFor('thread', 'b')).toEqual([]);
});

it('protects memberships first requested while a topic deletion is already pending', async () => {
  const store = useTopicsStore();
  let remote = [{ item_id: 'unseen', topic_id: 'topic' }];
  const queryGate = deferred<void>();
  backend.backendListTopicMemberships!.mockImplementation(async () => {
    const snapshot = [...remote];
    await queryGate.promise;
    return snapshot;
  });
  const deleteGate = deferred<void>();
  backend.backendDeleteTopic!.mockReturnValueOnce(deleteGate.promise);
  const remove = store.remove('topic');
  const read = store.loadMembershipsFor('thread', ['unseen']);
  await nextTick();
  remote = [];
  deleteGate.resolve();
  await remove;
  queryGate.resolve();
  await read;
  expect(store.topicIdsFor('thread', 'unseen')).toEqual([]);
});

it('allows membership loads and deletion retry after a pending topic delete fails', async () => {
  const store = useTopicsStore();
  let remote = [{ item_id: 'thread', topic_id: 'topic' }];
  backend.backendListTopicMemberships!.mockImplementation(async () => [...remote]);
  const deleteGate = deferred<void>();
  backend.backendDeleteTopic!.mockReturnValueOnce(deleteGate.promise);
  const remove = store.remove('topic');
  const rejected = expect(remove).rejects.toThrow('delete refused');
  const read = store.loadMembershipsFor('thread', ['thread']);
  deleteGate.reject(new Error('delete refused'));
  await Promise.all([rejected, read]);
  expect(store.topicIdsFor('thread', 'thread')).toEqual(['topic']);
  backend.backendDeleteTopic!.mockImplementationOnce(async () => { remote = []; });
  await store.remove('topic');
  await store.loadMembershipsFor('thread', ['thread']);
  expect(store.topicIdsFor('thread', 'thread')).toEqual([]);
});

it('links and deletes a large topic selection without nesting the call stack', async () => {
  const store = useTopicsStore();
  const ids = Array.from({ length: 5000 }, (_, index) => `bulk-${index}`);
  const receipts = ids.map((item_id) => ({ item_id, revision: item_id }));
  backend.backendAddTopicItems!.mockResolvedValue(receipts);
  backend.backendDeleteTopic!.mockResolvedValue(undefined);
  await expect(store.addItemsToTopic('topic', 'thread', ids)).resolves.toEqual(receipts);
  expect(store.topicIdsFor('thread', ids.at(-1)!)).toEqual(['topic']);
  await store.remove('topic');
  expect(store.topicIdsFor('thread', ids[0]!)).toEqual([]);
  expect(store.topicIdsFor('thread', ids.at(-1)!)).toEqual([]);
});

for (const outcome of ['success', 'failure'] as const) {
  it(`serializes deletion behind a delayed add ${outcome}, including finalizers and newly accessed ids`, async () => {
    const store = useTopicsStore();
    let remote = [{ item_id: 'unseen', topic_id: 'topic' }];
    const addGate = deferred<{ item_id: string; revision: string }[]>();
    backend.backendAddTopicItems!.mockReturnValueOnce(addGate.promise)
      .mockResolvedValueOnce([{ item_id: 'other-item', revision: 'other' }]);
    backend.backendDeleteTopic!.mockImplementationOnce(async () => { remote = []; });
    backend.backendListTopicMemberships!.mockImplementation(async () => [...remote]);
    const add = store.addItemsToTopic('topic', 'thread', ['thread']);
    const observed = outcome === 'failure' ? expect(add).rejects.toThrow('late error') : add;
    const remove = store.remove('topic');
    const read = store.loadMembershipsFor('thread', ['unseen']);
    await store.addItemsToTopic('other', 'thread', ['other-item']);
    expect(store.topicIdsFor('thread', 'other-item')).toEqual(['other']);
    expect(backend.backendDeleteTopic).not.toHaveBeenCalled();
    if (outcome === 'success') addGate.resolve([{ item_id: 'thread', revision: 'late' }]);
    else addGate.reject(new Error('late error'));
    await Promise.all([observed, remove, read]);
    expect(store.topicIdsFor('thread', 'thread')).toEqual([]);
    expect(store.topicIdsFor('thread', 'unseen')).toEqual([]);
    expect(store.topicIdsFor('thread', 'other-item')).toEqual(['other']);
  });
}

it('reports membership failure, keeps it unknown, and clears only the owned error after retry', async () => {
  const store = useTopicsStore();
  const { useConnectionStore } = await import('./connection');
  const connection = useConnectionStore();
  connection.reportBackendError('templates', new Error('templates offline'));
  backend.backendListTopicMemberships!.mockRejectedValueOnce(new Error('memberships offline'));
  await store.loadMembershipsFor('thread', ['one']);
  expect(store.hasMembershipsFor('thread', 'one')).toBe(false);
  expect(store.membershipsKnownFor('thread', ['one'])).toBe(false);
  expect(store.membershipsFailedFor('thread', ['one'])).toBe(true);
  expect(connection.backendErrorFor('topicMemberships')).toBeTruthy();
  backend.backendListTopicMemberships!.mockResolvedValueOnce([{ item_id: 'one', topic_id: 'topic' }]);
  connection.requestRetry();
  await vi.waitFor(() => expect(store.membershipsKnownFor('thread', ['one'])).toBe(true));
  expect(store.topicIdsFor('thread', 'one')).toEqual(['topic']);
  expect(connection.backendErrorFor('topicMemberships')).toBeNull();
  expect(connection.backendErrorFor('templates')).toBeTruthy();
});


it('retains Undo receipts from every successful removal in a partial batch', async () => {
  const store = useTopicsStore();
  const toast = useToast();
  toast.clear();
  const last = deferred<{ item_id: string; revision: string }>();
  backend.backendRemoveTopicItem!.mockImplementation(async ({ itemId }: { itemId: string }) => {
    if (itemId === 'b') throw new Error('remove refused');
    if (itemId === 'c') return last.promise;
    return { item_id: itemId, revision: 'revision-a' };
  });
  store.memberships = { 'thread:a': ['topic'], 'thread:b': ['topic'], 'thread:c': ['topic'] };
  const pending = useThreadActions().removeFromTopic(['a', 'b', 'c', 'a'], 'topic');
  await vi.waitFor(() => expect(backend.backendRemoveTopicItem).toHaveBeenCalledTimes(3));
  expect(toast.state.toasts).toHaveLength(0);
  last.resolve({ item_id: 'c', revision: 'revision-c' });
  await pending;
  expect(store.topicIdsFor('thread', 'a')).toEqual([]);
  expect(store.topicIdsFor('thread', 'b')).toEqual(['topic']);
  expect(store.topicIdsFor('thread', 'c')).toEqual([]);
  expect(toast.state.toasts).toHaveLength(1);
  expect(toast.state.toasts[0]?.tone).toBe('warning');
  expect(toast.state.toasts[0]?.message).toContain('2');
  expect(toast.state.toasts[0]?.message).toContain('1');
  backend.backendUndoTopicMemberships!.mockResolvedValueOnce(['a', 'c']);
  await toast.state.toasts[0]?.action?.handler();
  expect(backend.backendUndoTopicMemberships).toHaveBeenCalledExactlyOnceWith(
    'topic', 'thread', [{ item_id: 'a', revision: 'revision-a' }, { item_id: 'c', revision: 'revision-c' }], true,
  );
  toast.clear();
});

it('offers no Undo when a failed removal batch did not change any memberships', async () => {
  const toast = useToast();
  toast.clear();
  backend.backendRemoveTopicItem!.mockRejectedValue(new Error('remove refused'));
  await useThreadActions().removeFromTopic(['a', 'b'], 'topic');
  expect(toast.state.toasts).toHaveLength(1);
  expect(toast.state.toasts[0]?.tone).toBe('error');
  expect(toast.state.toasts[0]?.action).toBeUndefined();
  toast.clear();
});
