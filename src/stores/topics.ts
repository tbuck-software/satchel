import { orderedLoad } from './internal/orderedLoad';
import { useConnectionStore } from '@/stores/connection';
import { useAccountsStore } from '@/stores/accounts';
import { defineStore } from 'pinia';
import { computed, ref, watch } from 'vue';
import {
  backendAddTopicFile,
  backendAddTopicItems,
  backendCommandsAvailable,
  backendDeleteTopic,
  backendDeleteTopicFile,
  backendDeleteTopicNote,
  backendDownloadTopicFile,
  backendGetTopicTimeline,
  backendListTopicFiles,
  backendListTopicMemberships,
  backendListTopics,
  backendRemoveTopicItem,
  backendUndoTopicMemberships,
  type TopicMembershipReceipt,
  backendUpsertTopic,
  backendUpsertTopicNote,
  type BackendTopic,
  type BackendTopicFile,
  type BackendTopicTimelineItem,
  type TopicItemType,
} from '@/services/backend';

// Components take topic shapes from here, not from the service layer (ADR 0012).
export type {
  BackendTopic as Topic,
  BackendTopicFile as TopicFile,
  BackendTopicTimelineItem as TopicTimelineItem,
};

export interface TopicPickerTarget {
  itemType: Exclude<TopicItemType, 'event'>;
  itemIds: string[];
}

type TopicUpsertInput = Parameters<typeof backendUpsertTopic>[0];
type TopicCreateInput = Omit<TopicUpsertInput, 'id'>;

export interface OptimisticTopicCreation {
  topic: BackendTopic;
  persisted: Promise<BackendTopic>;
}

function membershipKey(itemType: TopicItemType, itemId: string): string {
  return `${itemType}:${itemId}`;
}

/** Register every reader guard before starting a bulk write, without nesting calls. */
function guardedWrite<T>(guards: ReturnType<typeof orderedLoad>[], write: () => Promise<T>): Promise<T> {
  let start!: () => void;
  const pending = new Promise<T>((resolve, reject) => {
    start = () => {
      try { resolve(write()); }
      catch (error) { reject(error); }
    };
  });
  const protectedWrites = Promise.all(guards.map((guard) => guard.mutate(() => pending)));
  start();
  return protectedWrites.then(() => pending);
}

export const useTopicsStore = defineStore('topics', () => {
  const accounts = useAccountsStore();
  const connection = useConnectionStore();
  watch(() => accounts.visibilityKey, () => {
    for (const topic of items.value) {
      topic.item_count = 0;
      topic.unread_count = 0;
      topic.last_sender_name = null;
      topic.last_sender_email = null;
    }
    void load();
  });
  const loader = orderedLoad(() => accounts.visibilityKey);
  const items = ref<BackendTopic[]>([]);
  const { loading } = loader;
  /** `${itemType}:${itemId}` -> topic ids shared by chips and picker. */
  const memberships = ref<Record<string, string[]>>({});
  const loadedMemberships = ref(new Set<string>());
  const membershipFailures = ref<Record<string, { itemType: TopicItemType; itemId: string }>>({});
  /** Non-null while the "add to topic" picker overlay is open. */
  const pickerTarget = ref<TopicPickerTarget | null>(null);
  /** Topic shown in the inbox detail pane instead of a thread; '' = none. */
  const activeTopicId = ref('');
  /** Concurrent creates from keyboard/pointer retries share one local topic and write. */
  const pendingCreates = new Map<string, OptimisticTopicCreation>();
  const pendingCreatesById = new Map<string, Promise<BackendTopic>>();
  const pendingTopicDeletes = new Set<Promise<void>>();

  const activeTopics = computed(() => items.value.filter((t) => t.status === 'active'));
  const finishedTopics = computed(() => items.value.filter((t) => t.status !== 'active'));

  function byId(id: string): BackendTopic | undefined {
    return items.value.find((t) => t.id === id);
  }

  async function load() {
    if (!backendCommandsAvailable()) return;
    await loader.load(backendListTopics, (rows) => { items.value = rows; },
      (error) => connection.reportBackendError('topics', error),
      () => connection.clearBackendError('topics'));
  }

  function reconcile(topic: BackendTopic) {
    const idx = items.value.findIndex((t) => t.id === topic.id);
    if (idx >= 0) items.value.splice(idx, 1, topic);
    else items.value.push(topic);
    return topic;
  }

  async function persist(input: TopicUpsertInput) {
    if (input.id) await pendingCreatesById.get(input.id);
    return loader.mutate(async () => reconcile(await backendUpsertTopic(input)));
  }

  function createKey(input: TopicCreateInput): string {
    return JSON.stringify([
      input.name.trim().toLowerCase(),
      input.description ?? null,
      input.color ?? null,
      input.icon ?? null,
      input.status ?? null,
      input.due_date ?? null,
    ]);
  }

  function create(input: TopicCreateInput): OptimisticTopicCreation {
    const key = createKey(input);
    const pending = pendingCreates.get(key);
    if (pending) return pending;

    const now = new Date().toISOString();
    const optimistic: BackendTopic = {
      id: `topic-${crypto.randomUUID()}`,
      name: input.name.trim(),
      description: input.description ?? '',
      color: input.color ?? null,
      icon: input.icon ?? null,
      status: input.status ?? 'active',
      due_date: input.due_date ?? null,
      item_count: 0,
      note_count: 0,
      unread_count: 0,
      last_sender_name: null,
      last_sender_email: null,
      created_at: now,
      updated_at: now,
    };
    items.value.push(optimistic);

    const persisted = loader.mutate(() => backendUpsertTopic({ ...input, id: optimistic.id })
      .then(reconcile)
      .catch((error) => {
        items.value = items.value.filter((topic) => topic.id !== optimistic.id);
        if (activeTopicId.value === optimistic.id) activeTopicId.value = '';
        throw error;
      })
      .finally(() => {
        pendingCreates.delete(key);
        pendingCreatesById.delete(optimistic.id);
      }));
    const creation = { topic: optimistic, persisted };
    pendingCreates.set(key, creation);
    pendingCreatesById.set(optimistic.id, persisted);
    return creation;
  }

  function upsert(input: TopicUpsertInput) {
    if (input.id != null) return persist(input);
    return create(input).persisted;
  }

  async function remove(id: string) {
    const pending = enqueueMembershipWrite(id, () => loader.mutate(async () => {
      const guards = [fileLoader(id), ...membershipLoads.values()];
      const remove = async () => {
        await backendDeleteTopic(id);
        items.value = items.value.filter((t) => t.id !== id);
        delete files.value[id];
        for (const key of Object.keys(memberships.value)) {
          memberships.value[key] = (memberships.value[key] ?? []).filter((topicId) => topicId !== id);
        }
      };
      membershipRevision++;
      await guardedWrite(guards, () => remove().finally(() => { membershipRevision++; }));
    }));
    pendingTopicDeletes.add(pending);
    void pending.finally(() => pendingTopicDeletes.delete(pending)).catch(() => {});
    return pending;
  }

  let membershipRevision = 0;
  // Per item guards allow disjoint chip/picker reads to finish independently.
  const membershipLoads = new Map<string, ReturnType<typeof orderedLoad>>();
  function membershipLoader(itemType: TopicItemType, itemId: string) {
    const key = membershipKey(itemType, itemId);
    if (!membershipLoads.has(key)) {
      const guard = orderedLoad();
      membershipLoads.set(key, guard);
      // Deletes also affect items first requested while the delete is pending.
      for (const pending of pendingTopicDeletes) {
        // The delete caller reports errors; this promise only delays readers.
        void guard.mutate(() => pending).catch(() => {});
      }
    }
    return membershipLoads.get(key)!;
  }

  async function loadMembershipsFor(itemType: TopicItemType, itemIds: string[]) {
    if (!backendCommandsAvailable() || itemIds.length === 0) return;
    let batch: ReturnType<typeof backendListTopicMemberships> | undefined;
    let batchRevision = -1;
    const read = () => {
      if (!batch || batchRevision !== membershipRevision) {
        batchRevision = membershipRevision;
        batch = backendListTopicMemberships(itemType, itemIds);
      }
      return batch;
    };
    await Promise.all(itemIds.map((id) => membershipLoader(itemType, id).load(
      read,
      (loaded) => {
        memberships.value[membershipKey(itemType, id)] = loaded
          .filter((row) => row.item_id === id).map((row) => row.topic_id);
        loadedMemberships.value.add(membershipKey(itemType, id));
      },
      (error) => {
        membershipFailures.value[membershipKey(itemType, id)] = { itemType, itemId: id };
        connection.reportBackendError('topicMemberships', error);
      },
      () => {
        delete membershipFailures.value[membershipKey(itemType, id)];
        if (Object.keys(membershipFailures.value).length === 0) connection.clearBackendError('topicMemberships');
      },
    )));
  }

  function membershipsFailedFor(itemType: TopicItemType, itemIds: string[]): boolean {
    return itemIds.some((id) => membershipKey(itemType, id) in membershipFailures.value);
  }

  function membershipsKnownFor(itemType: TopicItemType, itemIds: string[]): boolean {
    return itemIds.every((id) => hasMembershipsFor(itemType, id)) && !membershipsFailedFor(itemType, itemIds);
  }

  watch(() => connection.retryToken, () => {
    for (const { itemType, itemId } of Object.values(membershipFailures.value)) {
      void loadMembershipsFor(itemType, [itemId]);
    }
  });

  function hasMembershipsFor(itemType: TopicItemType, itemId: string): boolean {
    return loadedMemberships.value.has(membershipKey(itemType, itemId));
  }

  function topicIdsFor(itemType: TopicItemType, itemId: string): string[] {
    return memberships.value[membershipKey(itemType, itemId)] ?? [];
  }

  // Serialize writes to the same topic. Receipts come from SQLite, never from
  // a cached or failed membership lookup. Different topics remain independent.
  const membershipWrites = new Map<string, Promise<unknown>>();
  function enqueueMembershipWrite<T>(topicId: string, run: () => Promise<T>): Promise<T> {
    const previous = membershipWrites.get(topicId);
    const pending = previous ? previous.catch(() => {}).then(run) : run();
    membershipWrites.set(topicId, pending);
    void pending.finally(() => {
      if (membershipWrites.get(topicId) === pending) membershipWrites.delete(topicId);
    }).catch(() => {});
    return pending;
  }

  function writeMemberships<T>(topicId: string, itemType: TopicItemType, itemIds: string[], write: () => Promise<T>): Promise<T> {
    return enqueueMembershipWrite(topicId, () => loader.mutate(() => {
      membershipRevision++;
      // Invalidate the shared batch before any per-item guard releases readers.
      return guardedWrite(itemIds.map((id) => membershipLoader(itemType, id)),
        () => write().finally(() => { membershipRevision++; }));
    }));
  }

  async function addItemsToTopic(topicId: string, itemType: TopicItemType, itemIds: string[]): Promise<TopicMembershipReceipt[]> {
    const ids = [...new Set(itemIds)];
    return writeMemberships(topicId, itemType, ids, async () => {
      const added = await backendAddTopicItems({ topicId, itemType, itemIds: ids });
      for (const id of ids) {
        const key = membershipKey(itemType, id);
        memberships.value[key] = [...new Set([...(memberships.value[key] ?? []), topicId])];
      }
      const topic = byId(topicId);
      if (topic) topic.item_count += added.length;
      return added;
    });
  }

  async function removeItemFromTopic(topicId: string, itemType: TopicItemType, itemId: string): Promise<TopicMembershipReceipt | null> {
    return writeMemberships(topicId, itemType, [itemId], async () => {
      const removed = await backendRemoveTopicItem({ topicId, itemType, itemId });
      const key = membershipKey(itemType, itemId);
      memberships.value[key] = (memberships.value[key] ?? []).filter((id) => id !== topicId);
      const topic = byId(topicId);
      if (topic && removed) topic.item_count = Math.max(0, topic.item_count - 1);
      return removed;
    });
  }

  async function undoMemberships(topicId: string, itemType: TopicItemType,
    receipts: TopicMembershipReceipt[], restore: boolean): Promise<void> {
    await writeMemberships(topicId, itemType, receipts.map((row) => row.item_id), async () => {
      const changed = await backendUndoTopicMemberships(topicId, itemType, receipts, restore);
      for (const id of changed) {
        const key = membershipKey(itemType, id);
        const others = (memberships.value[key] ?? []).filter((id) => id !== topicId);
        memberships.value[key] = restore ? [...others, topicId] : others;
      }
      const topic = byId(topicId);
      if (topic) topic.item_count = Math.max(0, topic.item_count + (restore ? changed.length : -changed.length));
    });
    // A rejected stale receipt says nothing about the current membership.
    await loadMembershipsFor(itemType, receipts.map((row) => row.item_id));
  }

  async function fetchTimeline(topicId: string): Promise<BackendTopicTimelineItem[]> {
    if (!backendCommandsAvailable()) return [];
    return backendGetTopicTimeline(topicId);
  }

  async function upsertNote(input: Parameters<typeof backendUpsertTopicNote>[0]) {
    return loader.mutate(async () => {
      const note = await backendUpsertTopicNote(input);
      const topic = byId(input.topicId);
      if (topic && !input.id) topic.note_count += 1;
      return note;
    });
  }

  async function deleteNote(topicId: string, noteId: string) {
    return loader.mutate(async () => {
      await backendDeleteTopicNote(noteId);
      const topic = byId(topicId);
      if (topic) topic.note_count = Math.max(0, topic.note_count - 1);
    });
  }

  /**
   * Files stored on a topic, per topic.
   *
   * Held here rather than in the view that shows them: two views of the same
   * topic used to keep separate copies, so a file deleted in one stayed listed
   * in the other.
   */
  const files = ref<Record<string, BackendTopicFile[]>>({});
  const fileLoads = new Map<string, ReturnType<typeof orderedLoad>>();
  function fileLoader(topicId: string) {
    if (!fileLoads.has(topicId)) fileLoads.set(topicId, orderedLoad());
    return fileLoads.get(topicId)!;
  }
  const filesInFlight = new Map<string, Promise<void>>();

  function filesFor(topicId: string): BackendTopicFile[] {
    return files.value[topicId] ?? [];
  }

  function setFiles(topicId: string, next: BackendTopicFile[]) {
    files.value = { ...files.value, [topicId]: next };
  }

  /** Topic files use the command transport, including the browser mock. */
  function loadFiles(topicId: string): Promise<void> {
    if (!backendCommandsAvailable() || !topicId) return Promise.resolve();
    const running = filesInFlight.get(topicId);
    if (running) return running;
    const request = fileLoader(topicId).load(() => backendListTopicFiles(topicId),
      (loaded) => setFiles(topicId, loaded), (error) => { throw error; })
      .finally(() => filesInFlight.delete(topicId));
    filesInFlight.set(topicId, request);
    return request;
  }

  async function addFile(input: Parameters<typeof backendAddTopicFile>[0]): Promise<BackendTopicFile | null> {
    if (!backendCommandsAvailable()) return null;
    return fileLoader(input.topicId).mutate(async () => {
      const added = await backendAddTopicFile(input);
      setFiles(input.topicId, [...filesFor(input.topicId).filter((file) => file.id !== added.id), added]);
      return added;
    });
  }

  async function deleteFile(topicId: string, fileId: string) {
    return fileLoader(topicId).mutate(async () => {
      await backendDeleteTopicFile(fileId);
      setFiles(topicId, filesFor(topicId).filter((file) => file.id !== fileId));
    });
  }

  function downloadFile(fileId: string) {
    return backendDownloadTopicFile(fileId);
  }

  function openPicker(target: TopicPickerTarget) {
    if (target.itemIds.length === 0) return;
    pickerTarget.value = target;
    void loadMembershipsFor(target.itemType, target.itemIds);
  }

  function closePicker() {
    pickerTarget.value = null;
  }

  function openTopicDetail(id: string) {
    activeTopicId.value = id;
  }

  function closeTopicDetail() {
    activeTopicId.value = '';
  }

  return {
    items,
    loading,
    memberships,
    pickerTarget,
    activeTopicId,
    activeTopics,
    finishedTopics,
    byId,
    load,
    create,
    upsert,
    remove,
    loadMembershipsFor,
    topicIdsFor,
    hasMembershipsFor,
    membershipsKnownFor,
    membershipsFailedFor,
    undoMemberships,
    addItemsToTopic,
    removeItemFromTopic,
    fetchTimeline,
    upsertNote,
    deleteNote,
    files,
    filesFor,
    loadFiles,
    addFile,
    deleteFile,
    downloadFile,
    openPicker,
    closePicker,
    openTopicDetail,
    closeTopicDetail,
  };
});
