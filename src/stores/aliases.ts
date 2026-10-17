import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  backendDeleteAlias,
  backendListAliases,
  backendCommandsAvailable,
  backendUpsertAlias,
  type BackendAlias,
} from '@/services/backend';

export const useAliasesStore = defineStore('aliases', () => {
  const items = ref<BackendAlias[]>([]);
  const loading = ref(false);

  const byAccount = computed(() => {
    const map = new Map<string, BackendAlias[]>();
    for (const a of items.value) {
      const list = map.get(a.account_id) ?? [];
      list.push(a);
      map.set(a.account_id, list);
    }
    return map;
  });

  function aliasesFor(accountId: string): BackendAlias[] {
    return byAccount.value.get(accountId) ?? [];
  }

  async function load() {
    if (!backendCommandsAvailable()) return;
    loading.value = true;
    try {
      items.value = await backendListAliases();
    } finally {
      loading.value = false;
    }
  }

  async function upsert(input: Parameters<typeof backendUpsertAlias>[0]) {
    const alias = await backendUpsertAlias(input);
    const idx = items.value.findIndex((a) => a.id === alias.id);
    if (idx >= 0) items.value.splice(idx, 1, alias);
    else items.value.push(alias);
    return alias;
  }

  /** Resolves with the deleted alias so the caller can offer Undo. */
  async function remove(id: string): Promise<BackendAlias | null> {
    const alias = items.value.find((a) => a.id === id) ?? null;
    await backendDeleteAlias(id);
    items.value = items.value.filter((a) => a.id !== id);
    return alias;
  }

  /** Undo for a delete: the same id, so it survives a reload as itself. */
  async function restore(alias: BackendAlias) {
    await upsert({
      id: alias.id,
      accountId: alias.account_id,
      email: alias.email,
      displayName: alias.display_name ?? null,
      isDefault: alias.is_default === 1,
    });
  }

  return { items, byAccount, loading, aliasesFor, load, upsert, remove, restore };
});
