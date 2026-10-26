// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendContact } from '@/types/contacts';

beforeEach(() => vi.resetModules());

describe('mock contacts', () => {
  it('returns normalized correspondent records and searches them with the Rust command shape', async () => {
    const { sendCommand } = await import('./mock');
    const contacts = await sendCommand<BackendContact[]>('list_contacts');
    expect(contacts.length).toBeGreaterThan(0);
    expect(new Set(contacts.map((contact) => contact.email)).size).toBe(contacts.length);
    expect(contacts.every((contact) => contact.email === contact.email.trim().toLowerCase() && contact.frequency > 0)).toBe(true);
    const hits = await sendCommand<BackendContact[]>('search_contacts', { query: 'sarah', limit: 1 });
    expect(hits).toHaveLength(1);
    expect(hits[0]).toMatchObject({ display_name: 'Sarah Kim', avatar_locked: 0, notes: null });
    expect(await sendCommand('search_contacts', { query: '' })).toEqual([]);
  });

  it('persists profile creation and deletion through subsequent queries', async () => {
    const { sendCommand } = await import('./mock');
    const saved = await sendCommand<BackendContact>('upsert_contact', { input: {
      id: null, email: ' New@Example.com ', display_name: 'New Person', notes: 'A note', avatar_url: null,
    } });
    expect(saved.email).toBe('new@example.com');
    expect(await sendCommand('search_contacts', { query: 'new@example.com' })).toEqual([saved]);
    await sendCommand('delete_contact', { id: saved.id });
    expect(await sendCommand('search_contacts', { query: 'new@example.com' })).toEqual([]);
  });
});
