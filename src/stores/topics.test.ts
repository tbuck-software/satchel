import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  upsert: vi.fn(),
  listFiles: vi.fn(),
  addFile: vi.fn(),
  deleteFile: vi.fn(),
  native: vi.fn(() => true),
}));

vi.mock('@/services/backend', () => ({
  backendAddTopicItems: vi.fn(),
  backendDeleteTopic: vi.fn(),
  backendDeleteTopicNote: vi.fn(),
  backendGetTopicTimeline: vi.fn(),
  backendListTopicMemberships: vi.fn(),
  backendListTopics: vi.fn(),
  backendRemoveTopicItem: vi.fn(),
  backendUndoTopicMemberships: vi.fn(),
  backendCommandsAvailable: () => harness.native(),
  backendListTopicFiles: (...args: unknown[]) => harness.listFiles(...args),
  backendAddTopicFile: (...args: unknown[]) => harness.addFile(...args),
  backendDeleteTopicFile: (...args: unknown[]) => harness.deleteFile(...args),
  backendDownloadTopicFile: vi.fn(),
  backendUpsertTopic: (...args: unknown[]) => harness.upsert(...args),
  backendUpsertTopicNote: vi.fn(),
}));

import { backendAddTopicItems, backendRemoveTopicItem } from '@/services/backend';
import { useTopicsStore } from './topics';

const topic = {
  id: 'topic-1',
  name: 'Hunterport Quota Änderung',
  description: '',
  color: null,
  icon: null,
  status: 'active',
  due_date: null,
  item_count: 0,
  note_count: 0,
  unread_count: 0,
  last_sender_name: null,
  last_sender_email: null,
  created_at: '2026-08-11T07:00:00Z',
  updated_at: '2026-08-11T07:00:00Z',
};

describe('topics store creation', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    harness.upsert.mockReset();
  });

  it('adds one local topic immediately and shares its in-flight persistence', async () => {
    let finishCreate!: (value: typeof topic) => void;
    harness.upsert.mockImplementationOnce(
      () => new Promise<typeof topic>((resolve) => { finishCreate = resolve; }),
    );
    const store = useTopicsStore();

    const first = store.upsert({ name: topic.name });
    const second = store.upsert({ name: `  ${topic.name}  ` });

    expect(harness.upsert).toHaveBeenCalledTimes(1);
    expect(store.items).toHaveLength(1);
    expect(store.items[0]).toMatchObject({ name: topic.name, status: 'active' });
    const localId = store.items[0]!.id;
    expect(harness.upsert).toHaveBeenCalledWith({ name: topic.name, id: localId });

    const saved = { ...topic, id: localId };
    finishCreate(saved);
    await expect(Promise.all([first, second])).resolves.toEqual([saved, saved]);
    expect(store.items).toEqual([saved]);
  });

  it('removes the optimistic topic again when persistence fails', async () => {
    harness.upsert.mockRejectedValueOnce(new Error('db locked'));
    const store = useTopicsStore();

    const creation = store.create({ name: topic.name });

    expect(store.items).toContainEqual(creation.topic);
    await expect(creation.persisted).rejects.toThrow('db locked');
    expect(store.items).toEqual([]);
  });
});

// ARCH-14: topic files used to live in a component-local ref, so a second view
// of the same topic never learned about a delete.
describe('topic files', () => {
  const file = (id: string, topicId = 'topic-1') => ({
    id,
    topic_id: topicId,
    filename: `${id}.pdf`,
    mime_type: 'application/pdf',
    size: 10,
    created_at: '2026-08-11T07:00:00Z',
  });

  beforeEach(() => {
    setActivePinia(createPinia());
    harness.listFiles.mockReset();
    harness.addFile.mockReset();
    harness.deleteFile.mockReset();
    harness.native.mockReturnValue(true);
  });

  it('loads files once per topic, however many views ask at the same time', async () => {
    harness.listFiles.mockResolvedValue([file('a')]);
    const store = useTopicsStore();

    await Promise.all([store.loadFiles('topic-1'), store.loadFiles('topic-1')]);

    expect(harness.listFiles).toHaveBeenCalledTimes(1);
    expect(store.filesFor('topic-1').map((entry) => entry.id)).toEqual(['a']);
  });

  it('removes a deleted file for every consumer of the store', async () => {
    harness.listFiles.mockResolvedValue([file('a'), file('b')]);
    harness.deleteFile.mockResolvedValue(undefined);
    const detailView = useTopicsStore();
    const topicView = useTopicsStore();
    await detailView.loadFiles('topic-1');

    await detailView.deleteFile('topic-1', 'a');

    expect(topicView.filesFor('topic-1').map((entry) => entry.id)).toEqual(['b']);
  });

  it('keeps files per topic separate', async () => {
    harness.listFiles.mockImplementation(async (topicId: string) => [file(`${topicId}-file`, topicId)]);
    const store = useTopicsStore();

    await store.loadFiles('topic-1');
    await store.loadFiles('topic-2');
    harness.addFile.mockResolvedValue(file('new', 'topic-2'));
    await store.addFile({ topicId: 'topic-2', filename: 'new.pdf', contentBase64: 'AA==' });

    expect(store.filesFor('topic-1').map((entry) => entry.id)).toEqual(['topic-1-file']);
    expect(store.filesFor('topic-2').map((entry) => entry.id)).toEqual(['topic-2-file', 'new']);
  });

  it('does nothing without native commands', async () => {
    harness.native.mockReturnValue(false);
    const store = useTopicsStore();

    await store.loadFiles('topic-1');
    const added = await store.addFile({ topicId: 'topic-1', filename: 'x', contentBase64: 'AA==' });

    expect(harness.listFiles).not.toHaveBeenCalled();
    expect(harness.addFile).not.toHaveBeenCalled();
    expect(added).toBeNull();
    expect(store.filesFor('topic-1')).toEqual([]);
  });
});

describe('topics store memberships', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.mocked(backendAddTopicItems).mockReset();
    vi.mocked(backendRemoveTopicItem).mockReset();
  });

  it('reports only the items an add actually linked', async () => {
    const store = useTopicsStore();
    store.memberships['thread:member'] = ['topic-1'];
    vi.mocked(backendAddTopicItems).mockResolvedValueOnce([{ item_id: 'fresh', revision: 'one' }]);

    await expect(store.addItemsToTopic('topic-1', 'thread', ['member', 'fresh'])).resolves.toEqual([{ item_id: 'fresh', revision: 'one' }]);
    expect(store.topicIdsFor('thread', 'member')).toEqual(['topic-1']);
    expect(store.topicIdsFor('thread', 'fresh')).toEqual(['topic-1']);
  });

  it('rolls back only its own topic when an add fails', async () => {
    const store = useTopicsStore();
    let rejectAdd: (error: Error) => void = () => {};
    vi.mocked(backendAddTopicItems).mockImplementationOnce(
      () => new Promise((_, reject) => { rejectAdd = reject; }),
    );
    vi.mocked(backendAddTopicItems).mockResolvedValueOnce([{ item_id: 'thread-1', revision: 'one' }]);

    const failing = store.addItemsToTopic('topic-1', 'thread', ['thread-1']);
    await store.addItemsToTopic('topic-2', 'thread', ['thread-1']);
    rejectAdd(new Error('refused'));

    await expect(failing).rejects.toThrow('refused');
    expect(store.topicIdsFor('thread', 'thread-1')).toEqual(['topic-2']);
  });

  it('restores only its own topic when a removal fails', async () => {
    const store = useTopicsStore();
    store.memberships['thread:thread-1'] = ['topic-1'];
    let rejectRemove: (error: Error) => void = () => {};
    vi.mocked(backendRemoveTopicItem).mockImplementationOnce(
      () => new Promise((_, reject) => { rejectRemove = reject; }),
    );
    vi.mocked(backendAddTopicItems).mockResolvedValueOnce([{ item_id: 'thread-1', revision: 'one' }]);

    const failing = store.removeItemFromTopic('topic-1', 'thread', 'thread-1');
    await store.addItemsToTopic('topic-2', 'thread', ['thread-1']);
    rejectRemove(new Error('refused'));

    await expect(failing).rejects.toThrow('refused');
    expect([...store.topicIdsFor('thread', 'thread-1')].sort()).toEqual(['topic-1', 'topic-2']);
  });
});
