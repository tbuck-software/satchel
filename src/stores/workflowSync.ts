import { defineStore } from 'pinia';
import { ref } from 'vue';
import { backendCommandsAvailable, backendListLocalOnlyWorkflowAccounts } from '@/services/backend';

/**
 * Which accounts keep Done and Snooze on this device because their mail
 * server cannot store Satchel's keywords. Every other account carries both
 * through the server (ADR 0009). Read once per visit to the account settings.
 */
export const useWorkflowSyncStore = defineStore('workflowSync', () => {
  const localOnly = ref<string[]>([]);
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

  function isLocalOnly(accountId: string): boolean {
    return localOnly.value.includes(accountId);
  }

  return { localOnly, load, isLocalOnly };
});
