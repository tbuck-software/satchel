import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendCommandsAvailable,
  backendListAccountQuotas,
  type BackendAccountQuota,
} from '@/services/backend';
import { quotaLevel, type QuotaLevel } from '@/utils/mailboxQuota';

/**
 * The last mailbox quota each account's server reported. The backend reads
 * it during a full sync at most every few hours; this store only mirrors the
 * stored numbers, so reloading it after a sync is one cheap query.
 */
export const useAccountQuotaStore = defineStore('accountQuota', () => {
  const quotas = ref<BackendAccountQuota[]>([]);
  let request = 0;

  async function load() {
    if (!backendCommandsAvailable()) return;
    const current = ++request;
    try {
      const next = await backendListAccountQuotas();
      if (current !== request) return;
      if (JSON.stringify(next) !== JSON.stringify(quotas.value)) quotas.value = next;
    } catch {
      // A quota is a courtesy; failing to read it must not raise a banner.
      // The last list stays until the next load succeeds.
    }
  }

  function quotaFor(accountId: string): BackendAccountQuota | null {
    return quotas.value.find((quota) => quota.account_id === accountId) ?? null;
  }

  function levelFor(accountId: string): QuotaLevel | null {
    return quotaLevel(quotaFor(accountId));
  }

  return { quotas, load, quotaFor, levelFor };
});
