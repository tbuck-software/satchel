import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import {
  backendCancelFollowUp,
  backendListFollowUps,
  backendCommandsAvailable,
  backendUpsertFollowUp,
  type BackendFollowUp,
} from '@/services/backend';

export const useFollowUpsStore = defineStore('followUps', () => {
  const items = ref<BackendFollowUp[]>([]);
  const loading = ref(false);

  const byThread = computed(() => {
    const result = new Map<string, BackendFollowUp>();
    for (const item of items.value) {
      const previous = result.get(item.thread_id);
      if (!previous || item.status === 'due' || item.due_at < previous.due_at) {
        result.set(item.thread_id, item);
      }
    }
    return result;
  });

  async function load() {
    if (loading.value) return;
    if (!backendCommandsAvailable()) {
      items.value = [];
      return;
    }

    loading.value = true;
    try {
      const [pending, due] = await Promise.all([
        backendListFollowUps('pending'),
        backendListFollowUps('due'),
      ]);
      items.value = [...due, ...pending].sort((a, b) => a.due_at.localeCompare(b.due_at));
    } catch {
      // Reminder metadata is additive; a failed refresh must not make the mail
      // list fail or discard the last known due dates.
    } finally {
      loading.value = false;
    }
  }

  function forThread(threadId: string) {
    return byThread.value.get(threadId) ?? null;
  }

  /**
   * Set or move the reminder for a thread. Replaces the thread's existing
   * reminder instead of stacking a second one: the row shows exactly one.
   */
  async function schedule(threadId: string, dueAt: string, note?: string | null) {
    const existing = byThread.value.get(threadId);
    const saved = await backendUpsertFollowUp({
      id: existing?.id ?? null,
      threadId,
      dueAt,
      note: note ?? existing?.note ?? null,
    });
    items.value = [...items.value.filter((item) => item.id !== saved.id), saved]
      .sort((a, b) => a.due_at.localeCompare(b.due_at));
    return saved;
  }

  async function cancel(threadId: string) {
    const item = byThread.value.get(threadId);
    if (!item) return;
    await backendCancelFollowUp(item.id);
    items.value = items.value.filter((candidate) => candidate.id !== item.id);
  }

  return { items, loading, byThread, load, forThread, schedule, cancel };
});
