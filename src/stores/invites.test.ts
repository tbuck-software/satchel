import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backendImportIcs,
  backendListCalendarEvents,
  backendListCalendarSources,
  backendListInvites,
} from '@/services/backend';
import { useInvitesStore } from './invites';

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendImportIcs: vi.fn(),
  backendListInvites: vi.fn(),
  backendListCalendarEvents: vi.fn(),
  backendListCalendarSources: vi.fn(),
}));

const importMock = vi.mocked(backendImportIcs);
const listInvitesMock = vi.mocked(backendListInvites);
const listEventsMock = vi.mocked(backendListCalendarEvents);
const listSourcesMock = vi.mocked(backendListCalendarSources);

describe('invites store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    listInvitesMock.mockResolvedValue([]);
    listEventsMock.mockResolvedValue([]);
    listSourcesMock.mockResolvedValue([]);
  });

  it('imports a calendar file and reloads the local mirror', async () => {
    const invite = {
      id: 'invite-1',
      account_id: 'account-1',
      uid: 'uid-1',
      all_day: 0,
      rsvp_status: 'needs-action',
      raw_ics: 'BEGIN:VCALENDAR',
      created_at: '2026-08-30T00:00:00Z',
      updated_at: '2026-08-30T00:00:00Z',
      summary: 'Planning',
    } as never;
    importMock.mockResolvedValue(invite);
    listInvitesMock.mockResolvedValue([invite]);

    const invites = useInvitesStore();
    const result = await invites.importIcs({ raw: 'BEGIN:VCALENDAR', sourceFilename: 'plan.ics' });

    expect(result).toBe(invite);
    expect(importMock).toHaveBeenCalledWith({ raw: 'BEGIN:VCALENDAR', sourceFilename: 'plan.ics' });
    expect(listInvitesMock).toHaveBeenCalledOnce();
    expect(listEventsMock).toHaveBeenCalledOnce();
    expect(listSourcesMock).toHaveBeenCalledOnce();
    expect(invites.items).toEqual([invite]);
  });
});
