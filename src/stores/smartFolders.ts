import { useConnectionStore } from './connection';
import { orderedLoad } from './internal/orderedLoad';
import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendDeleteSmartFolder,
  backendListSmartFolders,
  backendCommandsAvailable,
  backendUpsertSmartFolder,
  type BackendSmartFolder,
} from '@/services/backend';

export const useSmartFoldersStore = defineStore('smartFolders', () => {
  const items = ref<BackendSmartFolder[]>([]);
  const connection = useConnectionStore();
  const loader = orderedLoad();
  const { loading, loadFailed } = loader;

  async function load() {
    if (!backendCommandsAvailable()) return;
    await loader.load(backendListSmartFolders, (rows) => { items.value = rows; },
      (error) => connection.reportBackendError('smartFolders', error),
      () => connection.clearBackendError('smartFolders'));
  }

  async function upsert(input: Parameters<typeof backendUpsertSmartFolder>[0]) {
    return loader.mutate(async () => {
      const folder = await backendUpsertSmartFolder(input);
      const idx = items.value.findIndex((f) => f.id === folder.id);
      if (idx >= 0) items.value.splice(idx, 1, folder);
      else items.value.push(folder);
      return folder;
    });
  }

  /** Resolves with the deleted folder so the caller can offer Undo. */
  async function remove(id: string): Promise<BackendSmartFolder | null> {
    return loader.mutate(async () => {
      const folder = items.value.find((f) => f.id === id) ?? null;
      await backendDeleteSmartFolder(id);
      items.value = items.value.filter((f) => f.id !== id);
      return folder;
    });
  }

  /** Undo for a delete: same id and same place in the sidebar order. */
  async function restore(folder: BackendSmartFolder) {
    await upsert({
      id: folder.id,
      name: folder.name,
      icon: folder.icon ?? null,
      query: folder.query,
      sortOrder: folder.sort_order,
    });
  }

  return { items, loading, loadFailed, load, upsert, remove, restore };
});
