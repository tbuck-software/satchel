import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendCommandsAvailable,
  backendListLocalOnlyWorkflowAccounts,
  type BackendLocalOnlyWorkflowAccount,
} from '@/services/backend';

/**
 * Which accounts keep Done and Snooze on this device, and why: a server
 * without custom keywords, Gmail over IMAP, or a server that refused the
 * last write (tried again later). Every other account carries both through
 * the server (ADR 0009). Read once per visit to the account settings.
 */
export const useWorkflowSyncStore = defineStore('workflowSync', () => {
  const localOnly = ref<BackendLocalOnlyWorkflowAccount[]>([]);
  let request = 0;

  async function load() {
    if (!backendCommandsAvailable()) return;
    const current = ++request;
    try {
      const next = await backendListLocalOnlyWorkflowAccounts();
      if (current === request) localOnly.value = next;
    } catch {
      // Informational only; the last answer stays until a load succeeds.
    }
  }

  function reasonFor(accountId: string): string | null {
    return localOnly.value.find((entry) => entry.account_id === accountId)?.reason ?? null;
  }

  function isLocalOnly(accountId: string): boolean {
    return reasonFor(accountId) !== null;
  }

  return { localOnly, load, reasonFor, isLocalOnly };
});
