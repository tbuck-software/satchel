import { defineStore } from 'pinia';
import { ref } from 'vue';
import {
  backendListInvites,
  backendListCalendarEvents,
  backendListCalendarSources,
  backendCreateCalendarEvent,
  backendDeleteCalendarEvent,
  backendImportIcs,
  backendCommandsAvailable,
  backendRespondToInvite,
  backendSyncGoogleCalendar,
  backendSyncGroupware,
  type BackendCalendarEvent,
  type BackendCalendarInvite,
  type BackendCalendarSource,
} from '@/services/backend';

export const useInvitesStore = defineStore('invites', () => {
  const items = ref<BackendCalendarInvite[]>([]);
  const events = ref<BackendCalendarEvent[]>([]);
  const sources = ref<BackendCalendarSource[]>([]);
  const loading = ref(false);

  async function load() {
    // "Can I send commands?", not "am I in a Tauri webview?" — the second is
    // false under the mock transport, which left the calendar empty there and
    // every surface reading from it untestable (AGENTS.md, Frontend).
    if (!backendCommandsAvailable()) return;
    loading.value = true;
    try {
      const [inviteRows, eventRows, sourceRows] = await Promise.all([
        backendListInvites(),
        backendListCalendarEvents(),
        backendListCalendarSources(),
      ]);
      items.value = inviteRows;
      events.value = eventRows;
      sources.value = sourceRows;
    } finally {
      loading.value = false;
    }
  }

  /**
   * Mirror the remote calendars (Google API und CalDAV) into the local events
   * table, then reload. Returns the collected error list so the caller can
   * surface it once — a missing scope should produce one toast, not a toast
   * per poll.
   */
  async function syncRemote(): Promise<string[]> {
    if (!backendCommandsAvailable()) return [];
    const [google, dav] = await Promise.allSettled([
      backendSyncGoogleCalendar(),
      backendSyncGroupware(),
    ]);
    const errors: string[] = [];
    let checked = 0;
    for (const result of [google, dav]) {
      if (result.status === 'fulfilled') {
        checked += result.value.accounts_checked;
        errors.push(...result.value.errors);
      } else {
        errors.push(result.reason instanceof Error ? result.reason.message : String(result.reason));
      }
    }
    if (checked > 0) await load();
    return errors;
  }

  async function createEvent(input: Parameters<typeof backendCreateCalendarEvent>[0]) {
    const event = await backendCreateCalendarEvent(input);
    const idx = events.value.findIndex((item) => item.id === event.id);
    if (idx >= 0) events.value.splice(idx, 1, event);
    else events.value.push(event);
    return event;
  }

  async function deleteEvent(id: string) {
    await backendDeleteCalendarEvent(id);
    events.value = events.value.filter((item) => item.id !== id);
  }

  /** Import a local calendar file and refresh the invite/event mirror. */
  async function importIcs(input: Parameters<typeof backendImportIcs>[0]) {
    const invite = await backendImportIcs(input);
    await load();
    return invite;
  }

  function findByMessageId(messageId: string): BackendCalendarInvite | null {
    return items.value.find((inv) => inv.message_id === messageId) ?? null;
  }

  async function respond(
    inviteId: string,
    response: 'accepted' | 'declined' | 'tentative',
    comment?: string,
  ) {
    await backendRespondToInvite({ inviteId, response, comment });
    const idx = items.value.findIndex((inv) => inv.id === inviteId);
    const existing = idx >= 0 ? items.value[idx] : undefined;
    if (idx >= 0 && existing) {
      items.value[idx] = { ...existing, rsvp_status: response };
    }
  }

  return {
    items,
    events,
    sources,
    loading,
    load,
    findByMessageId,
    respond,
    createEvent,
    deleteEvent,
    importIcs,
    syncRemote,
  };
});
