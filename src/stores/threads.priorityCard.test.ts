import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Thread } from '@/types/mail';
import { useThreadsStore } from './threads';

function thread(input: Partial<Thread> & Pick<Thread, 'id' | 'participants'>): Thread {
  return {
    id: input.id,
    accountId: 'acc-work',
    subject: input.subject ?? 'Letzter Kontext',
    participants: input.participants,
    sentParticipants: input.sentParticipants,
    snippet: input.snippet ?? 'Vorschau',
    lastMessageAt: input.lastMessageAt ?? new Date().toISOString(),
    lastMessageAtMs: Date.parse(input.lastMessageAt ?? new Date().toISOString()),
    messageCount: input.messageCount ?? 1,
    unreadCount: input.unreadCount ?? 0,
    hasAttachments: input.hasAttachments ?? false,
    workflowState: input.workflowState ?? 'inbox',
    priority: input.priority ?? 0,
    isStarred: input.isStarred ?? false,
    hasInvite: input.hasInvite ?? false,
    mailboxNames: input.mailboxNames ?? ['INBOX'],
    tags: input.tags ?? [],
    category: input.category,
  };
}

const mara = [{ name: 'Mara Vogel', email: 'mara@example.com' }];

describe('smart inbox priority card layout', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 7, 24, 12));
  });
  afterEach(() => vi.useRealTimers());

  it('leads with a capped priority card, weaves the folders into today', () => {
    const store = useThreadsStore();
    const at = (day: number, hour: number) => new Date(2026, 7, day, hour).toISOString();

    store.threads = [
      thread({ id: 'prio-newest', participants: mara, priority: 2, lastMessageAt: at(24, 11) }),
      thread({ id: 'prio-second', participants: mara, priority: 3, lastMessageAt: at(24, 10) }),
      thread({ id: 'prio-third', participants: mara, priority: 2, lastMessageAt: at(23, 9) }),
      thread({ id: 'prio-oldest', participants: mara, priority: 2, lastMessageAt: at(20, 9) }),
      thread({
        id: 'today-plain',
        participants: mara,
        sentParticipants: mara,
        lastMessageAt: at(24, 9),
        lastMessageAtMs: Date.parse(at(24, 9)),
      }),
      thread({
        id: 'notification',
        participants: [{ name: 'Build Bot', email: 'notifications@example.com' }],
        category: 'updates',
        lastMessageAt: at(24, 8),
        lastMessageAtMs: Date.parse(at(24, 8)),
      }),
      thread({
        id: 'yesterday-plain',
        participants: mara,
        sentParticipants: mara,
        lastMessageAt: at(23, 9),
        lastMessageAtMs: Date.parse(at(23, 9)),
      }),
    ];

    const entries = store.smartListEntries;
    expect(entries.map((entry) => entry.kind)).toEqual([
      'priority-card',
      'date-header',
      'thread',
      'folder-row',
      'date-header',
      'thread',
    ]);

    const card = entries[0];
    if (card?.kind !== 'priority-card') throw new Error('missing priority card');
    // Newest three as full rows, the oldest behind the footer.
    expect(card.threads.map((item) => item.id)).toEqual([
      'prio-newest',
      'prio-second',
      'prio-third',
    ]);
    expect(card.hiddenCount).toBe(1);
    expect(card.bucket.id).toBe('bucket-priority');

    // "Today" covers only the plain rows. The folders file their own mail.
    expect(entries[1]).toEqual(expect.objectContaining({
      kind: 'date-header',
      label: 'Today',
      threadIds: ['today-plain'],
    }));
    const folderRow = entries[3];
    if (folderRow?.kind !== 'folder-row') throw new Error('missing folder row');
    expect(folderRow.folders.map((folder) => folder.key)).toEqual(['notifications']);
    expect(entries[4]).toEqual(expect.objectContaining({ label: 'Yesterday', threadIds: ['yesterday-plain'] }));
  });

  it('keeps the "Today" header above the folders on a day without plain mail', () => {
    const store = useThreadsStore();

    store.threads = [
      thread({
        id: 'notification',
        participants: [{ name: 'Build Bot', email: 'notifications@example.com' }],
        category: 'updates',
        lastMessageAt: new Date(2026, 7, 24, 8).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 7, 24, 8).toISOString()),
      }),
      thread({
        id: 'yesterday-plain',
        participants: mara,
        sentParticipants: mara,
        lastMessageAt: new Date(2026, 7, 23, 9).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 7, 23, 9).toISOString()),
      }),
    ];

    const entries = store.smartListEntries;
    expect(entries.map((entry) => entry.kind)).toEqual([
      'date-header',
      'folder-row',
      'date-header',
      'thread',
    ]);
    // No priority mail means no card. The empty "Today" section offers no bulk action.
    expect(entries[0]).toEqual(expect.objectContaining({ label: 'Today', count: 0, threadIds: [] }));
  });

  it('shows all priority mail without a footer when the card is not full', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'prio-only',
        participants: mara,
        priority: 2,
        lastMessageAt: new Date(2026, 7, 24, 11).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 7, 24, 11).toISOString()),
      }),
    ];

    const card = store.smartListEntries[0];
    if (card?.kind !== 'priority-card') throw new Error('missing priority card');
    expect(card.threads.map((item) => item.id)).toEqual(['prio-only']);
    expect(card.hiddenCount).toBe(0);
  });
});
