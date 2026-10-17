import { createPinia, setActivePinia } from 'pinia';
import { effectScope, nextTick, watchEffect } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Thread } from '@/types/mail';
import { useSettingsStore } from './settings';
import { useThreadsStore } from './threads';

const { projection } = vi.hoisted(() => ({ projection: { calls: 0 } }));

vi.mock('@/utils/threadList', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/utils/threadList')>();
  return {
    ...actual,
    buildContactSummaries: (...args: Parameters<typeof actual.buildContactSummaries>) => {
      projection.calls += 1;
      return actual.buildContactSummaries(...args);
    },
  };
});

function thread(input: Partial<Thread> & Pick<Thread, 'id' | 'participants'>): Thread {
  return {
    id: input.id,
    accountId: input.accountId ?? 'acc-work',
    subject: input.subject ?? 'Letzter Kontext',
    participants: input.participants,
    sentParticipants: input.sentParticipants,
    snippet: input.snippet ?? 'Vorschau',
    lastMessageAt: input.lastMessageAt ?? '2026-08-24T15:08:00.000Z',
    lastMessageAtMs: Date.parse(input.lastMessageAt ?? '2026-08-24T15:08:00.000Z'),
    messageCount: input.messageCount ?? 1,
    unreadCount: input.unreadCount ?? 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    isStarred: false,
    hasInvite: false,
    mailboxNames: ['INBOX'],
    tags: [],
  };
}

function seed() {
  const store = useThreadsStore();
  store.threads = [
    thread({
      id: 'work',
      accountId: 'acc-work',
      subject: 'Projektstand',
      participants: [{ name: 'Mara Vogel', email: 'mara@example.com' }],
      sentParticipants: [{ name: 'Mara Vogel', email: 'mara@example.com' }],
    }),
    thread({
      id: 'private',
      accountId: 'acc-private',
      subject: 'Wochenende',
      participants: [{ name: 'Jon Falk', email: 'jon@example.com' }],
      sentParticipants: [{ name: 'Jon Falk', email: 'jon@example.com' }],
    }),
  ];
  return store;
}

describe('contact projections', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    projection.calls = 0;
  });

  it('rebuilds the contact summaries at most once per mutation', async () => {
    const store = seed();
    const scope = effectScope();
    // Everything the always-mounted sidebar reads.
    scope.run(() => watchEffect(() => {
      void store.contactSummaries.length;
      void store.pinnedContactSummaries.size;
    }));
    await nextTick();
    projection.calls = 0;

    store.markRead('work', true);
    await nextTick();

    // Was 2: the scoped summaries plus the unscoped variant behind the pins.
    expect(projection.calls).toBe(1);
    scope.stop();
  });

  it('resolves pinned contacts across accounts without the unscoped projection', () => {
    const store = seed();
    useSettingsStore().pinnedContacts = ['jon@example.com'];

    const pinned = store.pinnedContactSummaries.get('jon@example.com');

    expect(projection.calls).toBe(0);
    expect(pinned?.name).toBe('Jon Falk');
    expect(pinned?.previewText).toBe('Wochenende');
    expect(pinned?.threadIds).toEqual(['private']);
  });

  it('matches the unscoped summaries for a pinned address', () => {
    const store = seed();
    useSettingsStore().pinnedContacts = ['mara@example.com'];

    expect(store.pinnedContactSummaries.get('mara@example.com'))
      .toEqual(store.allContactSummaries.find((contact) => contact.email === 'mara@example.com'));
  });

  it('leaves unpinned contacts out of the pinned projection', () => {
    const store = seed();
    useSettingsStore().pinnedContacts = ['mara@example.com'];

    expect([...store.pinnedContactSummaries.keys()]).toEqual(['mara@example.com']);
  });
});
