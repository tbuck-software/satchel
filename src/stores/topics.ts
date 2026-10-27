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
  let loadRequest = 0;
  const items = ref<BackendTopic[]>([]);
  const loading = ref(false);
  /** `${itemType}:${itemId}` -> topic ids; optimistic source of truth for chips and picker. */
  const memberships = ref<Record<string, string[]>>({});
  /** Non-null while the "add to topic" picker overlay is open. */
  const pickerTarget = ref<TopicPickerTarget | null>(null);
  /** Topic shown in the inbox detail pane instead of a thread; '' = none. */
  const activeTopicId = ref('');
  /** Concurrent creates from keyboard/pointer retries share one local topic and write. */
  const pendingCreates = new Map<string, OptimisticTopicCreation>();
  const pendingCreatesById = new Map<string, Promise<BackendTopic>>();

  const activeTopics = computed(() => items.value.filter((t) => t.status === 'active'));
  const finishedTopics = computed(() => items.value.filter((t) => t.status !== 'active'));

  function byId(id: string): BackendTopic | undefined {
    return items.value.find((t) => t.id === id);
  }

  async function load() {
    if (!backendCommandsAvailable()) return;
    const request = ++loadRequest;
    const scope = accounts.visibilityKey;
    loading.value = true;
    try {
      const rows = await backendListTopics();
      if (request !== loadRequest || scope !== accounts.visibilityKey) return;
      items.value = rows;
      connection.clearBackendError('topics');
    } catch (error) {
      if (request === loadRequest && scope === accounts.visibilityKey) connection.reportBackendError('topics', error);
    } finally {
      if (request === loadRequest) loading.value = false;
    }
  }

  function reconcile(topic: BackendTopic) {
    const idx = items.value.findIndex((t) => t.id === topic.id);
    if (idx >= 0) items.value.splice(idx, 1, topic);
    else items.value.push(topic);
    return topic;
  }

  async function persist(input: TopicUpsertInput) {
    if (input.id) await pendingCreatesById.get(input.id);
    return reconcile(await backendUpsertTopic(input));
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

    const persisted = backendUpsertTopic({ ...input, id: optimistic.id })
      .then(reconcile)
      .catch((error) => {
        items.value = items.value.filter((topic) => topic.id !== optimistic.id);
        if (activeTopicId.value === optimistic.id) activeTopicId.value = '';
        throw error;
      })
      .finally(() => {
        pendingCreates.delete(key);
        pendingCreatesById.delete(optimistic.id);
      });
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
    await backendDeleteTopic(id);
    items.value = items.value.filter((t) => t.id !== id);
    for (const key of Object.keys(memberships.value)) {
      memberships.value[key] = (memberships.value[key] ?? []).filter((topicId) => topicId !== id);
    }
  }

  async function loadMembershipsFor(itemType: TopicItemType, itemIds: string[]) {
    if (!backendCommandsAvailable() || itemIds.length === 0) return;
    const rows = await backendListTopicMemberships(itemType, itemIds);
    const next: Record<string, string[]> = {};
    for (const id of itemIds) next[membershipKey(itemType, id)] = [];
    for (const row of rows) next[membershipKey(itemType, row.item_id)]?.push(row.topic_id);
    memberships.value = { ...memberships.value, ...next };
  }

  function hasMembershipsFor(itemType: TopicItemType, itemId: string): boolean {
    return membershipKey(itemType, itemId) in memberships.value;
  }

  function topicIdsFor(itemType: TopicItemType, itemId: string): string[] {
    return memberships.value[membershipKey(itemType, itemId)] ?? [];
  }

  /**
   * Link items to a topic. Resolves with the ids that were not members before,
   * so an undo can unlink exactly those and leave earlier memberships alone.
   */
  async function addItemsToTopic(topicId: string, itemType: TopicItemType, itemIds: string[]): Promise<string[]> {
    const added: string[] = [];
    for (const itemId of new Set(itemIds)) {
      const key = membershipKey(itemType, itemId);
      const current = memberships.value[key] ?? [];
      if (!current.includes(topicId)) {
        memberships.value[key] = [...current, topicId];
        added.push(itemId);
      }
    }
    try {
      await backendAddTopicItems({ topicId, itemType, itemIds });
      const topic = byId(topicId);
      if (topic) topic.item_count += added.length;
      return added;
    } catch (error) {
      // Roll back only this topic: another topic change on the same item may
      // have committed meanwhile.
      for (const itemId of added) {
        const key = membershipKey(itemType, itemId);
        memberships.value[key] = (memberships.value[key] ?? []).filter((id) => id !== topicId);
      }
      throw error;
    }
  }

  /** Unlink one item. Resolves with whether it was a member before. */
  async function removeItemFromTopic(topicId: string, itemType: TopicItemType, itemId: string): Promise<boolean> {
    const key = membershipKey(itemType, itemId);
    const wasMember = (memberships.value[key] ?? []).includes(topicId);
    memberships.value[key] = (memberships.value[key] ?? []).filter((id) => id !== topicId);
    try {
      await backendRemoveTopicItem({ topicId, itemType, itemId });
      const topic = byId(topicId);
      if (topic && wasMember) topic.item_count = Math.max(0, topic.item_count - 1);
      return wasMember;
    } catch (error) {
      const now = memberships.value[key] ?? [];
      if (wasMember && !now.includes(topicId)) memberships.value[key] = [...now, topicId];
      throw error;
    }
  }

  async function fetchTimeline(topicId: string): Promise<BackendTopicTimelineItem[]> {
    if (!backendCommandsAvailable()) return [];
    return backendGetTopicTimeline(topicId);
  }

  async function upsertNote(input: Parameters<typeof backendUpsertTopicNote>[0]) {
    const note = await backendUpsertTopicNote(input);
    const topic = byId(input.topicId);
    if (topic && !input.id) topic.note_count += 1;
    return note;
  }

  async function deleteNote(topicId: string, noteId: string) {
    await backendDeleteTopicNote(noteId);
    const topic = byId(topicId);
    if (topic) topic.note_count = Math.max(0, topic.note_count - 1);
  }

  /**
   * Files stored on a topic, per topic.
   *
   * Held here rather than in the view that shows them: two views of the same
   * topic used to keep separate copies, so a file deleted in one stayed listed
   * in the other.
   */
  const files = ref<Record<string, BackendTopicFile[]>>({});
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
    const request = backendListTopicFiles(topicId)
      .then((loaded) => setFiles(topicId, loaded))
      .finally(() => filesInFlight.delete(topicId));
    filesInFlight.set(topicId, request);
    return request;
  }

  async function addFile(input: Parameters<typeof backendAddTopicFile>[0]): Promise<BackendTopicFile | null> {
    if (!backendCommandsAvailable()) return null;
    const added = await backendAddTopicFile(input);
    setFiles(input.topicId, [...filesFor(input.topicId).filter((file) => file.id !== added.id), added]);
    return added;
  }

  async function deleteFile(topicId: string, fileId: string) {
    await backendDeleteTopicFile(fileId);
    setFiles(topicId, filesFor(topicId).filter((file) => file.id !== fileId));
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
