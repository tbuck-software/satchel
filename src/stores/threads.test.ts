import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Thread } from '@/types/mail';
import { useAccountsStore } from './accounts';
import { useSettingsStore } from './settings';
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

describe('threads store interactions', () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(() => vi.useRealTimers());

  it('opens a thread without entering bulk-selection mode', () => {
    const store = useThreadsStore();
    const participants = [{ name: 'Sender', email: 'sender@example.com' }];
    store.threads = [thread({ id: 'th-a', participants }), thread({ id: 'th-b', participants })];
    const target = store.threads[1]!.id;
    store.selectThread(store.threads[0]!.id);

    store.selectFromInteraction(target, { shiftKey: false, metaKey: false, ctrlKey: false }, true);

    expect(store.activeThreadId).toBe(target);
    expect(store.selectedThreadIds).toEqual([]);
  });

  it('suggests outbound people but not inbound-only bots', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'outbound',
        subject: 'Projektstand',
        participants: [{ name: 'Hotel Bot', email: 'bot@hotel.example' }],
        sentParticipants: [{ name: 'Mara Vogel', email: 'mara@example.com' }],
      }),
      thread({
        id: 'inbound-bot',
        participants: [{ name: 'Hotel Bot', email: 'bot@hotel.example' }],
        sentParticipants: [],
      }),
      thread({
        id: 'role-address',
        participants: [{ name: 'Example Hotel', email: 'info@hotel.example' }],
        sentParticipants: [{ name: 'Example Hotel', email: 'info@hotel.example' }],
      }),
    ];

    expect(store.contactSummaries.map((contact) => contact.email)).toEqual(['mara@example.com']);
    expect(store.contactSummaries[0]!.previewText).toBe('Projektstand');
  });

  it('reviews unknown senders regardless of category', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'unknown-person',
        participants: [{ name: 'Neue Person', email: 'person@example.com' }],
        lastMessageAt: '2026-08-24T15:08:00.000Z',
        lastMessageAtMs: Date.parse('2026-08-24T15:08:00.000Z'),
      }),
      thread({
        id: 'replied-person',
        participants: [{ name: 'Bekannte Person', email: 'known@example.com' }],
        sentParticipants: [{ name: 'Bekannte Person', email: 'known@example.com' }],
        lastMessageAt: '2026-08-24T14:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-08-24T14:00:00.000Z'),
      }),
      thread({
        id: 'newsletter',
        participants: [{ name: 'Daily News', email: 'daily@example.com' }],
        category: 'promotions',
        lastMessageAt: '2026-08-24T13:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-08-24T13:00:00.000Z'),
      }),
      thread({
        id: 'machine',
        participants: [{ name: 'Build Bot', email: 'notifications@example.com' }],
        lastMessageAt: '2026-08-24T12:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-08-24T12:00:00.000Z'),
      }),
    ];

    expect(store.newSenderStack?.senders.map((sender) => sender.senderEmail)).toEqual([
      'person@example.com',
      'daily@example.com',
    ]);
    const bucketThreadIds = store.smartInboxBuckets.flatMap((bucket) => bucket.threads.map((item) => item.id));
    expect(bucketThreadIds).toEqual(expect.arrayContaining(['machine']));
    expect(bucketThreadIds).not.toContain('newsletter');
    expect(store.smartListEntries.some((entry) => entry.kind === 'thread' && entry.thread.id === 'replied-person')).toBe(true);
  });

  it('uses focused-list precedence for priority, pins, invitations and notifications', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'priority-newsletter',
        participants: [{ name: 'Priority Shop', email: 'offers@shop.example' }],
        category: 'promotions',
        priority: 2,
      }),
      thread({
        id: 'pinned-notification',
        participants: [{ name: 'Pinned Bot', email: 'notifications@build.example' }],
        category: 'updates',
        isStarred: true,
      }),
      thread({
        id: 'calendar-newsletter',
        participants: [{ name: 'Calendar', email: 'calendar@example.com' }],
        category: 'promotions',
        hasInvite: true,
      }),
      thread({
        id: 'social-notification',
        participants: [{ name: 'GitHub', email: 'notifications@github.com' }],
        category: 'social',
      }),
    ];

    expect(
      Object.fromEntries(store.smartInboxBuckets.map((bucket) => [
        bucket.key,
        bucket.threads.map((item) => item.id),
      ])),
    ).toEqual({
      priority: ['priority-newsletter'],
      pinned: ['pinned-notification'],
      invitations: ['calendar-newsletter'],
      notifications: ['social-notification'],
    });
  });

  it('keeps a priority thread discoverable in the pinned bucket', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'priority-pin',
        participants: [{ name: 'Wichtige Person', email: 'important@example.com' }],
        priority: 2,
        isStarred: true,
      }),
    ];

    expect(store.smartInboxBuckets.find((bucket) => bucket.key === 'priority')?.threads)
      .toEqual([expect.objectContaining({ id: 'priority-pin' })]);
    expect(store.smartInboxBuckets.find((bucket) => bucket.key === 'pinned')?.threads)
      .toEqual([expect.objectContaining({ id: 'priority-pin' })]);
  });

  it('hides set-aside threads from Inbox while keeping their workflow view available', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'inbox-thread',
        participants: [{ name: 'Inbox', email: 'inbox@example.com' }],
      }),
      thread({
        id: 'aside-thread',
        participants: [{ name: 'Aside', email: 'aside@example.com' }],
        workflowState: 'aside' as never,
      }),
    ];
    store.workflowFilter = 'all';
    store.setMailboxFilter({ kind: 'section', value: 'inbox' });

    expect(store.visibleThreads.map((item) => item.id)).toEqual(['inbox-thread']);

    store.workflowFilter = 'aside' as never;
    store.setMailboxFilter({ kind: 'all', value: null });
    expect(store.visibleThreads.map((item) => item.id)).toEqual(['aside-thread']);
  });

  it('never reviews one of our own addresses as a new sender', () => {
    const accounts = useAccountsStore();
    accounts.accounts = [
      {
        id: 'acc-work',
        displayName: 'Ich',
        email: 'me@example.com',
        provider: 'imap',
        kind: 'email',
        color: '#000000',
      },
    ];
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'own-address',
        participants: [{ name: 'Ich', email: 'me@example.com' }],
      }),
    ];

    expect(store.newSenderStack).toBeNull();
  });

  it('suggests allowing a shared organisation domain but never a freemail domain', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'strato-info',
        participants: [{ name: 'Strato Info', email: 'info@strato.de' }],
      }),
      thread({
        id: 'strato-billing',
        participants: [{ name: 'Strato Rechnung', email: 'rechnung@strato.de' }],
      }),
      thread({
        id: 'gmail-one',
        participants: [{ name: 'Person Eins', email: 'person.eins@gmail.com' }],
      }),
      thread({
        id: 'gmail-two',
        participants: [{ name: 'Person Zwei', email: 'person.zwei@gmail.com' }],
      }),
    ];

    expect(store.newSenderStack?.domainSuggestion).toEqual({
      domain: 'strato.de',
      senderCount: 2,
      threadIds: expect.arrayContaining(['strato-info', 'strato-billing']),
    });
  });

  it('treats every sender of an accepted domain as known', () => {
    const settings = useSettingsStore();
    settings.acceptDomain('strato.de');
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'strato-info',
        participants: [{ name: 'Strato Info', email: 'info@strato.de' }],
      }),
      thread({
        id: 'other-person',
        participants: [{ name: 'Neue Person', email: 'person@example.com' }],
      }),
    ];

    expect(store.newSenderStack?.senders.map((sender) => sender.senderEmail)).toEqual(['person@example.com']);
  });

  it('hides deleted sender history but reviews a later new message again', () => {
    const settings = useSettingsStore();
    settings.setDismissedNewSender('person@example.com', '2026-07-11T09:00:00.000Z');
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'handled',
        participants: [{ name: 'Neue Person', email: 'person@example.com' }],
        lastMessageAt: '2026-07-11T08:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T08:00:00.000Z'),
      }),
      thread({
        id: 'new-again',
        participants: [{ name: 'Neue Person', email: 'person@example.com' }],
        lastMessageAt: '2026-07-11T10:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T10:00:00.000Z'),
      }),
    ];

    expect(store!.newSenderStack?.senders[0]!.threadIds).toEqual(['new-again']);
  });

  it('bundles matching subject patterns across intervening senders at the first thread position', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'app-store-9',
        subject: 'App Store Connect: Version 0.2.0 (9) has completed processing',
        participants: [{ name: 'App Store Connect', email: 'no_reply@email.apple.com' }],
        lastMessageAt: '2026-07-11T12:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T12:00:00.000Z'),
      }),
      thread({
        id: 'app-store-7',
        subject: 'App Store Connect: Version 0.1.1 (7) has completed processing',
        participants: [{ name: 'App Store Connect', email: 'no_reply@email.apple.com' }],
        lastMessageAt: '2026-07-11T11:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T11:00:00.000Z'),
      }),
      thread({
        id: 'testflight',
        subject: 'ALEX MORGAN has invited you to test Example App',
        participants: [{ name: 'ALEX MORGAN via TestFlight', email: 'no_reply@apple.com' }],
        lastMessageAt: '2026-07-11T10:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T10:00:00.000Z'),
      }),
      thread({
        id: 'app-store-1',
        subject: 'App Store Connect: Version 0.1.0 (1) has completed processing',
        participants: [{ name: 'App Store Connect', email: 'no_reply@email.apple.com' }],
        lastMessageAt: '2026-07-11T09:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T09:00:00.000Z'),
      }),
    ];

    // Date headers are layout, not what is under test here.
    const rows = store.listEntries.filter((entry) => entry.kind !== 'date-header');
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      kind: 'group',
      group: {
        senderName: 'App Store Connect',
        count: 3,
        threads: [{ id: 'app-store-9' }, { id: 'app-store-7' }, { id: 'app-store-1' }],
      },
    });
    expect(rows[1]).toMatchObject({ kind: 'thread', thread: { id: 'testflight' } });
  });

  it('keeps pipeline membership and group identity stable while checking off inbox rows', () => {
    useSettingsStore().setInboxGroupingMode('classic');
    const store = useThreadsStore();
    const sender = [{ name: 'CODE.EXAMPLE', email: 'gitlab@code.example' }];
    store.threads = [
      thread({ id: 'pipeline-new', participants: sender, subject: 'CLI | Failed pipeline for main | d7ade544' }),
      thread({ id: 'unrelated', participants: sender, subject: 'Other project | Merge request (!350)' }),
      thread({ id: 'pipeline-middle', participants: sender, subject: 'CLI | Failed pipeline for main | 6d404a27' }),
      thread({ id: 'pipeline-old', participants: sender, subject: 'CLI | Failed pipeline for main | 15acb27a' }),
    ];
    const bundle = () => store.listEntries.find((entry) => entry.kind === 'group');
    const before = bundle();
    expect(before?.group.threads.map((item) => item.id)).toEqual([
      'pipeline-new', 'pipeline-middle', 'pipeline-old',
    ]);
    store.applyWorkflowStates([{ threadId: 'unrelated', state: 'done' }]);
    expect(bundle()?.group.threads.map((item) => item.id)).toEqual([
      'pipeline-new', 'pipeline-middle', 'pipeline-old',
    ]);
    store.applyWorkflowStates([{ threadId: 'pipeline-new', state: 'done' }]);
    expect(bundle()?.group.id).toBe(before?.group.id);
    expect(bundle()?.group.threads.map((item) => item.id)).toEqual(['pipeline-middle', 'pipeline-old']);
  });

  it('opens a matching subject bundle inside a smart folder', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'notification-new',
        participants: [{ name: 'App Store Connect', email: 'no-reply@email.apple.com' }],
        category: 'updates',
        lastMessageAt: '2026-07-11T12:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T12:00:00.000Z'),
      }),
      thread({
        id: 'personal-between',
        participants: [{ name: 'Lena Lehmann', email: 'lena@example.com' }],
        category: 'personal',
        lastMessageAt: '2026-07-11T11:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T11:00:00.000Z'),
      }),
      thread({
        id: 'notification-old',
        participants: [{ name: 'App Store Connect', email: 'no-reply@email.apple.com' }],
        category: 'updates',
        lastMessageAt: '2026-07-11T10:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T10:00:00.000Z'),
      }),
    ];

    store.openSmartBucket('bucket-notifications');
    const groupEntry = store.smartListEntries.find((entry) => entry.kind === 'group');
    expect(groupEntry).toBeDefined();
    if (!groupEntry || groupEntry.kind !== 'group') return;

    store.openSimilarGroup(groupEntry.group.id);

    expect(store.activeSimilarGroup?.threads.map((item) => item.id)).toEqual([
      'notification-new',
      'notification-old',
    ]);
  });

  it('opens a CI project bundle without hiding issues or adding freshly synced runs to the open bundle', async () => {
    const store = useThreadsStore();
    const ciThread = (id: string, subject: string) => thread({
      id, subject, category: 'updates',
      participants: [{ name: 'Alex Morgan', email: 'notifications@github.com' }],
      lastMessageAt: '2026-10-07T10:00:00Z',
    });
    store.threads = [
      ciThread('failed', '[owner/repo] Run failed: CI - main (abcdef0)'),
      ciThread('issue', '[owner/repo] New issue: Pipeline failed (#123)'),
      ciThread('fixed', '[owner/repo] Run succeeded: Security audit - main (abcdef1)'),
    ];
    store.openSmartBucket('bucket-notifications');
    const rows = store.smartListEntries.filter((entry) => entry.kind !== 'date-header');
    expect(rows.map((entry) => entry.kind)).toEqual(['group', 'thread']);
    const bundle = rows[0];
    if (bundle?.kind !== 'group') throw new Error('Expected CI project bundle');
    expect(bundle.group.ciProject).toBe('owner/repo');
    store.openSimilarGroup(bundle.group.id);
    store.threads = [
      ciThread('new-run', '[owner/repo] Run failed: CI - feat/mail (abcdef2)'),
      ...store.threads,
    ];
    await nextTick();
    expect(store.activeSimilarGroup?.threads.map((item) => item.id)).toEqual(['failed', 'fixed']);
    store.applyWorkflowStates([{ threadId: 'failed', state: 'done' }]);
    await nextTick();
    expect(store.activeSimilarGroup?.ciProject).toBe('owner/repo');
    expect(store.activeSimilarGroup?.threads.map((item) => item.id)).toEqual(['fixed']);
  });

  it('keeps an opened sender bundle visible until its last thread is done', async () => {
    const settings = useSettingsStore();
    settings.setInboxGroupingMode('classic');
    const store = useThreadsStore();
    const sender = [{ name: 'App Store Connect', email: 'no_reply@email.apple.com' }];
    store.threads = [
      thread({
        id: 'notification-new',
        participants: sender,
        lastMessageAt: '2026-07-11T12:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T12:00:00.000Z'),
      }),
      thread({
        id: 'notification-middle',
        participants: sender,
        lastMessageAt: '2026-07-11T11:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T11:00:00.000Z'),
      }),
      thread({
        id: 'notification-old',
        participants: sender,
        lastMessageAt: '2026-07-11T10:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T10:00:00.000Z'),
      }),
    ];

    const groupEntry = store.listEntries.find((entry) => entry.kind === 'group');
    expect(groupEntry).toBeDefined();
    if (!groupEntry || groupEntry.kind !== 'group') return;

    store.openSimilarGroup(groupEntry.group.id);
    const openedGroupId = store.activeSimilarGroupId;

    store.applyWorkflowStates([{ threadId: 'notification-new', state: 'done' }]);
    await nextTick();
    expect(store.activeSimilarGroupId).toBe(openedGroupId);
    expect(store.activeSimilarGroup?.threads.map((item) => item.id)).toEqual([
      'notification-middle',
      'notification-old',
    ]);

    store.applyWorkflowStates([{ threadId: 'notification-middle', state: 'done' }]);
    await nextTick();
    expect(store.activeSimilarGroupId).toBe(openedGroupId);
    expect(store.activeSimilarGroup?.threads.map((item) => item.id)).toEqual([
      'notification-old',
    ]);

    store.applyWorkflowStates([{ threadId: 'notification-old', state: 'done' }]);
    await nextTick();
    expect(store.activeSimilarGroupId).toBe('');
    expect(store.activeSimilarGroup).toBeUndefined();
  });

  it('splits an opened smart folder into date sections for bulk actions', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 6, 16, 12));
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'today-note',
        participants: [{ name: 'Build Bot', email: 'notifications@example.com' }],
        category: 'updates',
        lastMessageAt: new Date(2026, 6, 16, 9).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 6, 16, 9).toISOString()),
      }),
      thread({
        id: 'old-note',
        participants: [{ name: 'Deploy Bot', email: 'no-reply@example.org' }],
        category: 'updates',
        lastMessageAt: new Date(2026, 5, 1, 9).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 5, 1, 9).toISOString()),
      }),
    ];

    store.openSmartBucket('bucket-notifications');

    const headers = store.smartListEntries.filter((entry) => entry.kind === 'date-header');
    expect(headers).toEqual([
      expect.objectContaining({ label: 'Today', threadIds: ['today-note'] }),
      expect.objectContaining({ label: 'June', threadIds: ['old-note'] }),
    ]);
  });

  it('keeps a smart folder open until its last thread is done', async () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'notification-new',
        participants: [{ name: 'Build Bot', email: 'notifications@example.com' }],
        category: 'updates',
        lastMessageAt: '2026-07-11T12:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T12:00:00.000Z'),
      }),
      thread({
        id: 'notification-old',
        participants: [{ name: 'Deploy Bot', email: 'no-reply@example.org' }],
        category: 'updates',
        lastMessageAt: '2026-07-11T10:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T10:00:00.000Z'),
      }),
    ];

    store.openSmartBucket('bucket-notifications');
    store.applyWorkflowStates([{ threadId: 'notification-new', state: 'done' }]);
    await nextTick();
    expect(store.activeSmartBucketId).toBe('bucket-notifications');
    expect(store.activeSmartBucket?.threads.map((item) => item.id)).toEqual(['notification-old']);

    // Only the final mail closes the folder and returns to the inbox.
    store.applyWorkflowStates([{ threadId: 'notification-old', state: 'done' }]);
    await nextTick();
    expect(store.activeSmartBucketId).toBe('');
    expect(store.activeSmartBucket).toBeUndefined();
  });

  it('opens a sender bundle created after smart folders are removed from the inbox list', () => {
    const settings = useSettingsStore();
    settings.setInboxGroupingMode('smart');
    const store = useThreadsStore();
    const lena = [{ name: 'Lena Lehmann', email: 'lena@example.com' }];

    store.threads = [
      thread({
        id: 'lena-new',
        participants: lena,
        sentParticipants: lena,
        category: 'personal',
        lastMessageAt: '2026-07-11T12:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T12:00:00.000Z'),
      }),
      thread({
        id: 'notification-between',
        participants: [{ name: 'Build Bot', email: 'notifications@example.com' }],
        category: 'updates',
        lastMessageAt: '2026-07-11T11:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T11:00:00.000Z'),
      }),
      thread({
        id: 'lena-old',
        participants: lena,
        sentParticipants: lena,
        category: 'personal',
        lastMessageAt: '2026-07-11T10:00:00.000Z',
        lastMessageAtMs: Date.parse('2026-07-11T10:00:00.000Z'),
      }),
    ];

    const groupEntry = store.smartListEntries.find((entry) => entry.kind === 'group');
    expect(groupEntry).toBeDefined();
    if (!groupEntry || groupEntry.kind !== 'group') return;

    store.openSimilarGroup(groupEntry.group.id);

    expect(store.activeSimilarGroup?.threads.map((item) => item.id)).toEqual([
      'lena-new',
      'lena-old',
    ]);
  });

  it('groups folder threads into rolling date sections and descending months', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 15, 12));
    const store = useThreadsStore();
    const atNoon = (year: number, month: number, day: number) => new Date(year, month, day, 12).toISOString();
    const participant = [{ name: 'Projekt', email: 'projekt@example.com' }];

    store.threads = [
      thread({ id: 'today', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2026, 9, 15) }),
      thread({ id: 'yesterday', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2026, 9, 14) }),
      thread({ id: 'last-week-new', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2026, 9, 13) }),
      thread({ id: 'last-week-old', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2026, 9, 8) }),
      thread({ id: 'october', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2026, 9, 7) }),
      thread({ id: 'september', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2026, 8, 1) }),
      thread({ id: 'august', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2026, 7, 1) }),
      thread({ id: 'previous-year', participants: participant, mailboxNames: ['Projekte'], lastMessageAt: atNoon(2025, 7, 1), priority: 3 }),
    ];
    store.mailboxFilter = { kind: 'folder', value: 'Projekte' };

    const headers = store.listEntries.filter((entry) => entry.kind === 'date-header');
    expect(headers).toEqual([
      expect.objectContaining({ label: 'Today', threadIds: ['today'] }),
      expect.objectContaining({ label: 'Yesterday', threadIds: ['yesterday'] }),
      expect.objectContaining({ label: 'Last week', threadIds: ['last-week-new', 'last-week-old'] }),
      expect.objectContaining({ id: 'date-month-2026-10', label: 'October', threadIds: ['october'] }),
      expect.objectContaining({ id: 'date-month-2026-09', label: 'September', threadIds: ['september'] }),
      expect.objectContaining({ id: 'date-month-2026-08', label: 'August', threadIds: ['august'] }),
      expect.objectContaining({ id: 'date-month-2025-08', label: 'August 2025', threadIds: ['previous-year'] }),
    ]);
  });

  it('keeps hidden pinned and set-aside threads out of smart month actions', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 8, 15, 12));
    const store = useThreadsStore();
    const settings = useSettingsStore();
    settings.inboxGroupingMode = 'smart';
    const august = new Date(2026, 7, 4, 12).toISOString();

    store.threads = [
      thread({
        id: 'visible-august',
        participants: [{ name: 'Visible', email: 'visible@example.com' }],
        sentParticipants: [{ name: 'Visible', email: 'visible@example.com' }],
        lastMessageAt: august,
        lastMessageAtMs: Date.parse(august),
      }),
      thread({
        id: 'pinned-august',
        participants: [{ name: 'Pinned', email: 'pinned@example.com' }],
        sentParticipants: [{ name: 'Pinned', email: 'pinned@example.com' }],
        lastMessageAt: august,
        lastMessageAtMs: Date.parse(august),
        isStarred: true,
      }),
      thread({
        id: 'aside-august',
        participants: [{ name: 'Aside', email: 'aside@example.com' }],
        sentParticipants: [{ name: 'Aside', email: 'aside@example.com' }],
        lastMessageAt: august,
        lastMessageAtMs: Date.parse(august),
        workflowState: 'aside' as never,
      }),
    ];

    const augustHeader = store.smartListEntries.find(
      (entry) => entry.kind === 'date-header' && entry.id === 'date-month-2026-08',
    );

    expect(augustHeader).toEqual(expect.objectContaining({
      threadIds: ['visible-august'],
    }));
    expect(store.smartInboxBuckets.find((bucket) => bucket.key === 'pinned')?.threadIds)
      .toContain('pinned-august');
    expect(store.visibleThreads.map((item) => item.id)).not.toContain('aside-august');
  });

  it('includes the local calendar day exactly seven days ago in last week', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 7, 3, 12));
    const store = useThreadsStore();
    const participant = [{ name: 'Projekt', email: 'projekt@example.com' }];

    store.threads = [
      thread({
        id: 'seven-days-ago',
        participants: participant,
        lastMessageAt: new Date(2026, 6, 27, 12).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 6, 27, 12).toISOString()),
      }),
      thread({
        id: 'eight-days-ago',
        participants: participant,
        lastMessageAt: new Date(2026, 6, 26, 12).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 6, 26, 12).toISOString()),
      }),
    ];

    const headers = store.listEntries.filter((entry) => entry.kind === 'date-header');
    expect(headers).toEqual([
      expect.objectContaining({ label: 'Last week', threadIds: ['seven-days-ago'] }),
      expect.objectContaining({ label: 'July', threadIds: ['eight-days-ago'] }),
    ]);
  });

  it('keeps sender bundles inside their folder date group when opened', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 6, 16, 12));
    const store = useThreadsStore();
    const participant = [{ name: 'Projekt', email: 'projekt@example.com' }];

    store.threads = [
      thread({
        id: 'today-new',
        participants: participant,
        mailboxNames: ['Projekte'],
        lastMessageAt: new Date(2026, 6, 16, 12).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 6, 16, 12).toISOString()),
      }),
      thread({
        id: 'today-old',
        participants: participant,
        mailboxNames: ['Projekte'],
        lastMessageAt: new Date(2026, 6, 16, 11).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 6, 16, 11).toISOString()),
      }),
      thread({
        id: 'yesterday-new',
        participants: participant,
        mailboxNames: ['Projekte'],
        lastMessageAt: new Date(2026, 6, 15, 12).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 6, 15, 12).toISOString()),
      }),
      thread({
        id: 'yesterday-old',
        participants: participant,
        mailboxNames: ['Projekte'],
        lastMessageAt: new Date(2026, 6, 15, 11).toISOString(),
        lastMessageAtMs: Date.parse(new Date(2026, 6, 15, 11).toISOString()),
      }),
    ];
    store.mailboxFilter = { kind: 'folder', value: 'Projekte' };

    const groups = store.listEntries.filter((entry) => entry.kind === 'group');
    expect(groups).toHaveLength(2);
    const yesterdayGroup = groups[1];
    if (!yesterdayGroup || yesterdayGroup.kind !== 'group') return;

    store.openSimilarGroup(yesterdayGroup.group.id);

    expect(store.activeSimilarGroup?.threads.map((item) => item.id)).toEqual([
      'yesterday-new',
      'yesterday-old',
    ]);
  });

  it('keeps waiting conversations out of Inbox and available in the waiting workflow', () => {
    const store = useThreadsStore();
    store.threads = [
      thread({
        id: 'needs-reply',
        participants: [{ name: 'Mike', email: 'mara@example.com' }],
        workflowState: 'inbox',
        mailboxNames: ['INBOX'],
      }),
      thread({
        id: 'awaiting-reply',
        participants: [{ name: 'Alex', email: 'alex@example.com' }],
        workflowState: 'waiting',
        mailboxNames: ['INBOX', 'SENT'],
      }),
    ];

    expect(store.visibleThreads.map((item) => item.id)).toEqual(['needs-reply']);

    store.workflowFilter = 'waiting';
    store.mailboxFilter = { kind: 'all', value: null };
    expect(store.visibleThreads.map((item) => item.id)).toEqual(['awaiting-reply']);
  });
});

describe('thread ordering', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('orders newest first and lifts priority mails above their peers', () => {
    const store = useThreadsStore();
    const zoe = [{ name: 'Zoe', email: 'zoe@example.com' }];
    store.threads = [
      thread({ id: 'older', participants: zoe, lastMessageAt: '2026-07-05T00:00:00Z' }),
      thread({ id: 'old-important', participants: zoe, priority: 2, lastMessageAt: '2026-07-01T00:00:00Z' }),
      thread({ id: 'newest', participants: zoe, lastMessageAt: '2026-07-09T00:00:00Z' }),
    ];
    store.mailboxFilter = { kind: 'all', value: null };

    expect(store.visibleThreads.map((t) => t.id)).toEqual(['old-important', 'newest', 'older']);
  });
});
