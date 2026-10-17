import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import {
  backendDeleteContact,
  backendListContacts,
  backendSearchContacts,
  backendSetContactAvatar,
  backendSyncContactAvatars,
  backendSyncLocalContactAvatars,
  backendCommandsAvailable,
  backendUpsertContact,
  type BackendContact,
} from '@/services/backend';

export interface ContactAvatarSyncSummary {
  accounts_checked: number;
  avatars_updated: number;
  fallback_avatars_updated: number;
  errors: string[];
}

export const useContactsStore = defineStore('contacts', () => {
  const contacts = ref<BackendContact[]>([]);
  const loaded = ref(false);
  const loading = ref(false);
  /** Why the address book is empty, when it is empty for a reason. */
  const error = ref<unknown>(null);
  const avatarSyncRunning = ref(false);
  let localAvatarRefreshRunning = false;
  const lastAvatarSync = ref<ContactAvatarSyncSummary | null>(null);
  const lastAvatarSyncError = ref('');

  const byEmail = computed(() => {
    const map = new Map<string, BackendContact>();
    for (const contact of contacts.value) {
      map.set(contact.email.trim().toLowerCase(), contact);
    }
    return map;
  });

  /**
   * Fill the address book, and say so when it could not be filled.
   *
   * The catch used to swallow every failure and set `loaded` anyway, on the
   * grounds that web and mock mode have no contacts backend. That reasoning
   * only covers those two: on a real backend the same branch turned a failed
   * query into an address book that looks imported and empty.
   */
  async function load() {
    loading.value = true;
    try {
      contacts.value = await backendListContacts();
      loaded.value = true;
      error.value = null;
    } catch (err) {
      if (!backendCommandsAvailable()) {
        // Web mode and local mock mode fall back to derived avatars.
        loaded.value = true;
        error.value = null;
        return;
      }
      error.value = err;
      loaded.value = false;
    } finally {
      loading.value = false;
    }
  }

  async function search(query: string, limit = 12): Promise<BackendContact[]> {
    if (!backendCommandsAvailable()) return [];
    return backendSearchContacts(query, limit);
  }

  async function syncAvatars(allowThirdParty = false): Promise<ContactAvatarSyncSummary | null> {
    if (avatarSyncRunning.value) {
      return lastAvatarSync.value;
    }
    avatarSyncRunning.value = true;
    lastAvatarSyncError.value = '';
    try {
      const result = await backendSyncContactAvatars(allowThirdParty);
      lastAvatarSync.value = result;
      if (result.avatars_updated + result.fallback_avatars_updated > 0) {
        await load();
      }
      return result;
    } catch (err) {
      lastAvatarSyncError.value = err instanceof Error ? err.message : String(err);
      return null;
    } finally {
      avatarSyncRunning.value = false;
    }
  }

  async function refreshLocalAvatars(): Promise<number> {
    if (localAvatarRefreshRunning || !backendCommandsAvailable()) return 0;
    localAvatarRefreshRunning = true;
    try {
      const updated = await backendSyncLocalContactAvatars();
      if (updated > 0) await load();
      return updated;
    } catch {
      // This is an optional background refresh. Manual import reports failures.
      return 0;
    } finally {
      localAvatarRefreshRunning = false;
    }
  }

  async function setAvatar(input: {
    email: string;
    displayName?: string | null;
    avatarUrl?: string | null;
    locked?: boolean | null;
  }): Promise<BackendContact> {
    // Throws: the caller says what failed, which a bare `null` could not.
    const contact = await backendSetContactAvatar(input);
    const index = contacts.value.findIndex(
      (item) => item.email.trim().toLowerCase() === contact.email.trim().toLowerCase(),
    );
    if (index >= 0) {
      contacts.value[index] = contact;
    } else {
      contacts.value = [contact, ...contacts.value];
    }
    return contact;
  }

  async function saveProfile(input: {
    email: string;
    displayName: string;
    notes: string;
  }): Promise<BackendContact> {
    const normalized = input.email.trim().toLowerCase();
    const previous = contacts.value.find(
      (item) => item.email.trim().toLowerCase() === normalized,
    );
    const now = new Date().toISOString();
    const optimistic: BackendContact = {
      id: previous?.id ?? `local-${normalized}`,
      email: input.email.trim(),
      display_name: input.displayName.trim() || null,
      avatar_url: previous?.avatar_url ?? null,
      avatar_source: previous?.avatar_source ?? null,
      avatar_locked: previous?.avatar_locked ?? 0,
      avatar_updated_at: previous?.avatar_updated_at ?? null,
      frequency: previous?.frequency ?? 0,
      last_contacted: previous?.last_contacted ?? null,
      notes: input.notes.trim() || null,
      created_at: previous?.created_at ?? now,
      updated_at: now,
    };
    const previousContacts = contacts.value;
    contacts.value = previous
      ? contacts.value.map((item) => item === previous ? optimistic : item)
      : [optimistic, ...contacts.value];

    if (!backendCommandsAvailable()) return optimistic;
    try {
      const saved = await backendUpsertContact({
        id: previous?.id,
        email: optimistic.email,
        displayName: optimistic.display_name,
        avatarUrl: optimistic.avatar_url,
        notes: optimistic.notes,
      });
      contacts.value = contacts.value.map((item) =>
        item.email.trim().toLowerCase() === normalized ? saved : item,
      );
      return saved;
    } catch (error) {
      contacts.value = previousContacts;
      throw error;
    }
  }

  /**
   * Drop a stored contact. Removes it optimistically and puts it back when the
   * backend refuses, so a failed delete never leaves the list lying.
   */
  async function remove(email: string): Promise<void> {
    const normalized = email.trim().toLowerCase();
    const existing = byEmail.value.get(normalized);
    if (!existing) return;

    const previousContacts = contacts.value;
    contacts.value = contacts.value.filter((item) => item !== existing);
    if (!backendCommandsAvailable()) return;
    try {
      await backendDeleteContact(existing.id);
    } catch (error) {
      contacts.value = previousContacts;
      throw error;
    }
  }

  function findByEmail(email?: string | null): BackendContact | null {
    const normalized = email?.trim().toLowerCase();
    if (!normalized) {
      return null;
    }
    return byEmail.value.get(normalized) ?? null;
  }

  return {
    contacts,
    loaded,
    loading,
    error,
    avatarSyncRunning,
    lastAvatarSync,
    lastAvatarSyncError,
    byEmail,
    load,
    search,
    syncAvatars,
    refreshLocalAvatars,
    setAvatar,
    saveProfile,
    remove,
    findByEmail,
  };
});
