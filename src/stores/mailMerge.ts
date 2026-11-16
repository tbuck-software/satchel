import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { backendCommandsAvailable } from '@/services/backend/core';
import {
  backendControlMailMergeJob,
  backendCreateMailMergeJob,
  backendGetMailMergeJob,
  backendListMailMergeJobs,
  backendParseMailMergeCsv,
  backendPreviewMailMerge,
} from '@/services/backend/mailMerge';
import type { BackendMergeJob, BackendMergeJobAction, BackendMergeJobDetail } from '@/types/mailMerge';

/** How often the panel looks while a job sends; events say when a copy settles. */
export const MAIL_MERGE_POLL_MS = 5000;

/**
 * Mail merge: reading a list, the preview, starting a job, and the jobs as
 * the Outbox shows them with the recipient rows of the one that is open. The
 * runner in Rust does the sending; this store only reads and steers.
 */
export const useMailMergeStore = defineStore('mailMerge', () => {
  const jobs = ref<BackendMergeJob[]>([]);
  const detail = ref<BackendMergeJobDetail | null>(null);
  const loadedOnce = ref(false);
  const error = ref<string | null>(null);
  const active = computed(() => jobs.value.some((job) => job.status === 'running'));

  let loadRequest = 0;
  async function load() {
    if (!backendCommandsAvailable()) return;
    const request = ++loadRequest;
    try {
      const next = await backendListMailMergeJobs();
      if (request !== loadRequest) return;
      jobs.value = next;
      loadedOnce.value = true;
      error.value = null;
      const open = detail.value?.job.id;
      if (open) {
        if (next.some((job) => job.id === open)) await openDetail(open);
        else detail.value = null;
      }
    } catch (err) {
      if (request !== loadRequest) return;
      error.value = err instanceof Error ? err.message : String(err);
    }
  }

  async function openDetail(id: string) {
    detail.value = await backendGetMailMergeJob(id);
  }

  function closeDetail() {
    detail.value = null;
  }

  async function control(id: string, action: BackendMergeJobAction) {
    const updated = await backendControlMailMergeJob(id, action);
    if (updated) {
      jobs.value = jobs.value.map((job) => (job.id === id ? updated : job));
    } else {
      jobs.value = jobs.value.filter((job) => job.id !== id);
      if (detail.value?.job.id === id) detail.value = null;
    }
    await load();
  }

  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let watchers = 0;

  /**
   * Keeps the list current while a view shows it. Counted, so two views can.
   * The view forwards `mail-merge:updated` to `load`; the poll keeps the
   * "next message at" line honest between events.
   */
  async function watch() {
    watchers += 1;
    if (watchers > 1) return;
    pollTimer = setInterval(() => {
      if (active.value) void load();
    }, MAIL_MERGE_POLL_MS);
    await load();
  }

  function unwatch() {
    watchers = Math.max(0, watchers - 1);
    if (watchers > 0) return;
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = null;
  }

  return {
    jobs, detail, loadedOnce, error, active, load, openDetail, closeDetail, control, watch, unwatch,
    parseCsv: backendParseMailMergeCsv,
    preview: backendPreviewMailMerge,
    createJob: backendCreateMailMergeJob,
  };
});
