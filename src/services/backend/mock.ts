import { splitSubjectPrefixes } from '@/utils/composerText';
/**
 * Stand-in command transport for `dev:mock` and the browser E2E suite.
 *
 * `vite.config.ts` aliases `@/services/backend/transport` onto this file when
 * `VITE_SATCHEL_MOCK` is set. Everything above the transport — services,
 * stores, composables, components — runs the exact code it runs in the app;
 * only the far side of `invoke` is different. That is the whole point: the
 * suite used to skip the backend call it was meant to prove.
 *
 * The fixtures live in `@/data/mock`, which nothing else may import.
 */
import { assertCommandPayload } from './commandContract';
import {
  mockAccounts,
  mockCalendarEvents,
  mockCalendarSources,
  mockDrafts,
  mockMessagesByThread,
  mockOutboxItems,
  mockThreads,
  mockTopics,
  mockTopicTimeline,
} from '@/data/mock';
import type { Account, Message, Thread } from '@/types/mail';
import type { BackendDraft } from '@/types/composer';
import type { BackendMessage, BackendMessageBody } from '@/types/messages';
import type { BackendCalendarEvent, BackendCalendarSource } from '@/types/calendar';
import type { BackendOutboxItem } from '@/types/outbox';
import type { BackendTopic, BackendTopicTimelineItem } from '@/types/topics';
import type { WorkflowState } from '@/types/workflow';
import { classifySmartBucket } from '@/utils/threadList';

export function backendCommandsAvailable(): boolean {
  return true;
}

type Payload = Record<string, unknown>;
type Handler = (payload: Payload) => unknown;

/* -------------------------------------------------------------------------- */
/* Mutable state                                                              */
/* -------------------------------------------------------------------------- */

const accounts: Account[] = mockAccounts.map((account) => ({ ...account }));
const threads: Thread[] = mockThreads.map((thread) => ({ ...thread }));
const messages: Record<string, Message[]> = Object.fromEntries(
  Object.entries(mockMessagesByThread).map(([id, list]) => [id, list.map((m) => ({ ...m }))]),
);
const outbox: BackendOutboxItem[] = mockOutboxItems.map((item) => ({ ...item }));
/*
 * `?mockLongThread=120` adds one HTML conversation of that many messages on
 * top of the Inbox, and `?mockExtraThreads=40` that many short threads below
 * it. Both exist for the reader's performance measurements.
 */
function searchNumber(name: string, maximum = 1_000): number {
  if (typeof window === 'undefined') return 0;
  const raw = Number(new URLSearchParams(window.location.search).get(name) ?? 0);
  return Number.isFinite(raw) ? Math.min(Math.max(Math.trunc(raw), 0), maximum) : 0;
}

function seedMeasurementThreads() {
  const longCount = searchNumber('mockLongThread');
  const now = Date.now();
  const people = [
    { name: 'Sarah Kim', email: 'sarah@app.io' },
    { name: 'Tom', email: 'tom@satchel.dev' },
    { name: 'Jonas Weber', email: 'jonas@partner.example' },
  ];
  if (longCount > 0) {
    const id = 'th-long';
    const list: Message[] = Array.from({ length: longCount }, (_, index) => {
      const from = people[index % people.length]!;
      const paragraphs = Array.from({ length: 6 }, (_unused, line) =>
        `<p>Reply ${index + 1}, paragraph ${line + 1}: the rollout plan for the payment `
        + 'service needs another look before Friday. Please check the numbers in the table below.</p>').join('');
      return {
        id: `m-long-${index + 1}`,
        threadId: id,
        from,
        to: [people[(index + 1) % people.length]!],
        subject: 'Rollout plan for the payment service',
        bodyText: `Reply ${index + 1}: the rollout plan for the payment service needs another look before Friday.`,
        bodyHtml: `<div>${paragraphs}<table><tr><td>Region</td><td>Share</td></tr>`
          + `<tr><td>EU</td><td>${index % 100}%</td></tr></table></div>`,
        timestamp: new Date(now - (longCount - index) * 15 * 60_000).toISOString(),
        isRead: true,
        attachments: [],
      };
    });
    messages[id] = list;
    threads.push({
      id,
      accountId: 'acc-work',
      subject: 'Rollout plan for the payment service',
      participants: people,
      snippet: list.at(-1)!.bodyText,
      lastMessageAt: new Date(now - 60_000).toISOString(),
      lastMessageAtMs: now - 60_000,
      messageCount: longCount,
      unreadCount: 0,
      hasAttachments: false,
      workflowState: 'inbox',
      priority: 1,
      mailboxNames: ['INBOX'],
      tags: [],
    });
  }
  const extraCount = searchNumber('mockExtraThreads', 20_000);
  for (let index = 0; index < extraCount; index += 1) {
    const id = `th-extra-${index + 1}`;
    const at = now - (2 + index) * 60 * 60_000;
    const from = people[index % 2 === 0 ? 0 : 2]!;
    messages[id] = [1, 2, 3].map((position) => ({
      id: `m-extra-${index + 1}-${position}`,
      threadId: id,
      from,
      to: [people[1]!],
      subject: `Status update ${index + 1}`,
      bodyText: `Update ${index + 1}.${position}`,
      bodyHtml: `<p>Update ${index + 1}.${position}: numbers attached.</p>`,
      timestamp: new Date(at - (3 - position) * 60_000).toISOString(),
      isRead: position < 3,
      attachments: [],
    }));
    threads.push({
      id,
      accountId: 'acc-work',
      subject: `Status update ${index + 1}`,
      participants: [from],
      snippet: `Update ${index + 1}.3`,
      lastMessageAt: new Date(at).toISOString(),
      lastMessageAtMs: at,
      messageCount: 3,
      unreadCount: 1,
      hasAttachments: false,
      // Done keeps them in one flat, dated list instead of the Inbox buckets.
      workflowState: 'done',
      senderKnown: true,
      priority: 1,
      mailboxNames: ['INBOX'],
      tags: [],
    });
  }
}
seedMeasurementThreads();

/** Commands answered so far, by name. Read by the reader's measurement script. */
const commandCounts: Record<string, number> = {};
if (typeof window !== 'undefined') {
  (window as typeof window & { __satchelMockCommands?: Record<string, number> })
    .__satchelMockCommands = commandCounts;
}

const drafts: BackendDraft[] = mockDrafts.map((draft) => ({ ...draft }));
const queuedDrafts = new Map<string, BackendDraft>();
for (const item of outbox.filter((entry) => entry.status === 'scheduled')) {
  const draft = drafts.find((entry) => entry.account_id === item.account_id && entry.subject === item.subject);
  if (draft) queuedDrafts.set(item.id, { ...draft });
}
const topics: BackendTopic[] = mockTopics.map((topic) => ({ ...topic }));
const topicTimeline: Record<string, BackendTopicTimelineItem[]> = { ...mockTopicTimeline };
function draftFromInput(scope: Record<string, unknown>): BackendDraft {
  return {
    ...scope,
    id: (scope.id as string | null) ?? nextId('draft'),
    account_id: scope.account_id as string, thread_id: (scope.thread_id as string | null) ?? null,
    to: (scope.to as string[] | undefined) ?? [], cc: (scope.cc as string[] | undefined) ?? [], bcc: (scope.bcc as string[] | undefined) ?? [],
    subject: (scope.subject as string | undefined) ?? '', body: (scope.body as string | undefined) ?? '',
    body_html: (scope.body_html as string | null) ?? null, scheduled_for: (scope.scheduled_for as string | null) ?? null,
    attachments: (scope.attachments as BackendDraft['attachments']) ?? [], updated_at: new Date().toISOString(),
  };
}

function putDraft(draft: BackendDraft) {
  const index = drafts.findIndex((entry) => entry.id === draft.id);
  const revision = (index >= 0 ? drafts[index]!.revision ?? 0 : 0) + 1;
  const stored = { ...draft, revision };
  if (index >= 0) drafts.splice(index, 1, stored);
  else drafts.unshift(stored);
  return { ...stored };
}

function assertDraftRevision(id: unknown, expected: unknown) {
  if (typeof id !== 'string' || typeof expected !== 'number') return;
  const current = drafts.find((entry) => entry.id === id);
  if (!current) throw new Error('backend.errors.draftGoneElsewhere');
  if ((current.revision ?? 0) !== expected) throw new Error('backend.errors.draftChangedElsewhere');
}

function assertDraftNotQueued(id: unknown) {
  for (const [queuedId, draft] of queuedDrafts) {
    if (draft.id !== id) continue;
    const item = outbox.find((entry) => entry.id === queuedId);
    if (item?.status !== 'failed') throw new Error('backend.errors.draftSendPending');
    outbox.splice(outbox.indexOf(item), 1);
    queuedDrafts.delete(queuedId);
  }
}

function takeQueuedDraft(id: string): BackendDraft {
  const index = outbox.findIndex((item) => item.id === id && ['pending', 'scheduled', 'failed'].includes(item.status));
  const draft = queuedDrafts.get(id);
  if (index < 0 || !draft) throw new Error('backend.errors.outboxActionCannotBeDiscarded');
  outbox.splice(index, 1);
  queuedDrafts.delete(id);
  return putDraft({ ...draft, scheduled_for: null });
}

const calendarEvents: BackendCalendarEvent[] = mockCalendarEvents.map((event) => ({ ...event }));
const calendarSources: BackendCalendarSource[] = mockCalendarSources.map((source) => ({ ...source }));
const senderPreferences = new Map<string, { category: string | null; isPriority: boolean }>();

/** A 2x2 slate-grey PNG. The smallest thing a thumbnail can be drawn from. */
const MOCK_PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGOYNa0HiBggFAAw1gbxw9+yowAAAABJRU5ErkJggg==';

let sequence = 0;
function nextId(prefix: string): string {
  sequence += 1;
  return `${prefix}-mock-${sequence}`;
}

/* -------------------------------------------------------------------------- */
/* Forced failures                                                            */
/* -------------------------------------------------------------------------- */

/**
 * `?backendFailure=send_message,cancel_outbox_action` makes those commands
 * reject with a backend-shaped error.
 *
 * This is the negative half of the send coverage: a test that only asserts the
 * happy path cannot tell a call that happened from a call that was skipped.
 * With the switch on, the same test must see the failure surface in the UI.
 */
function forcedFailures(): Set<string> {
  if (typeof window === 'undefined') return new Set();
  const raw = new URLSearchParams(window.location.search).get('backendFailure');
  return new Set((raw ?? '').split(',').map((value) => value.trim()).filter(Boolean));
}

/* -------------------------------------------------------------------------- */
/* DTO conversion                                                             */
/* -------------------------------------------------------------------------- */

function threadDto(thread: Thread) {
  return {
    id: thread.id,
    account_id: thread.accountId,
    subject: thread.subject,
    normalized_subject: splitSubjectPrefixes(thread.subject).base.replace(/\s+/g, ' '),
    snippet: thread.snippet,
    last_message_date: thread.lastMessageAt,
    sort_date: thread.sortDate ?? thread.lastMessageAt,
    message_count: thread.messageCount,
    workflow_state: thread.workflowState,
    unread_count: thread.unreadCount,
    has_attachments: thread.hasAttachments,
    has_draft: Boolean(thread.hasDraft),
    draft_snippet: thread.draftSnippet ?? null,
    priority: thread.priority,
    board_position: thread.boardPosition ?? null,
    tags: thread.tags ?? [],
    assignee: thread.assignee ?? null,
    shared_with: thread.sharedWith ?? [],
    participants: JSON.stringify(thread.participants),
    sent_participants: thread.sentParticipants
      ? JSON.stringify(thread.sentParticipants)
      : undefined,
    mailbox_names: JSON.stringify(thread.mailboxNames ?? []),
    category: thread.category ?? null,
    smart_bucket: classifySmartBucket(thread),
    is_muted: Boolean(thread.isMuted),
    is_starred: Boolean(thread.isStarred),
    snooze_until: thread.snoozeUntil ?? null,
    sender_known: Boolean(thread.senderKnown),
    has_invite: Boolean(thread.hasInvite),
    invite_status: thread.inviteStatus ?? null,
    invite_cancelled: Boolean(thread.inviteCancelled),
  };
}

function accountDto(account: Account) {
  return {
    id: account.id,
    display_name: account.displayName,
    email: account.email,
    provider: account.provider,
    kind: account.kind,
    color: account.color,
    last_oauth_error: account.lastOauthError ?? null,
    last_oauth_error_at: account.lastOauthErrorAt ?? null,
    last_sync_at: account.lastSyncAt ?? null,
    sync_status: account.syncStatus ?? null,
    last_sync_error: account.lastSyncError ?? null,
    last_sync_error_at: account.lastSyncErrorAt ?? null,
    sync_progress_json: account.syncProgressJson ?? null,
  };
}

function messageDto(message: Message): BackendMessage {
  return {
    id: message.id,
    thread_id: message.threadId,
    subject: message.subject,
    snippet: message.bodyText.slice(0, 180),
    from_address: message.from.email,
    from_name: message.from.name,
    to_addresses: message.to.map((p) => p.email),
    cc_addresses: (message.cc ?? []).map((p) => p.email),
    reply_to: message.replyTo ?? null,
    date: message.timestamp,
    // Like `list_messages_preview`: no body; `list_message_bodies` brings it.
    body_text: message.bodyText.slice(0, 180),
    body_html: null,
    body_loaded: false,
    is_read: message.isRead,
    is_draft: Boolean(message.isDraft),
    list_unsubscribe: message.listUnsubscribe ?? null,
    list_unsubscribe_post: message.listUnsubscribePost ?? null,
    auth_spf: message.authSpf ?? null,
    auth_dkim: message.authDkim ?? null,
    auth_dmarc: message.authDmarc ?? null,
  };
}

function messageBodyDto(message: Message): BackendMessageBody {
  return {
    id: message.id,
    snippet: message.bodyText.slice(0, 180),
    body_text: message.bodyText,
    body_html: message.bodyHtml ?? null,
    body_loaded: true,
    auth_spf: message.authSpf ?? null,
    auth_dkim: message.authDkim ?? null,
    auth_dmarc: message.authDmarc ?? null,
  };
}

/* -------------------------------------------------------------------------- */
/* Helpers                                                                    */
/* -------------------------------------------------------------------------- */

function input(payload: Payload): Payload {
  return (payload.input as Payload | undefined) ?? {};
}

function ids(payload: Payload, key = 'thread_ids'): string[] {
  return (input(payload)[key] as string[] | undefined) ?? [];
}

function findThread(threadId: string): Thread | undefined {
  return threads.find((thread) => thread.id === threadId);
}

function byDateDesc(a: Thread, b: Thread): number {
  return new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime();
}

/* -------------------------------------------------------------------------- */
/* Command table                                                              */
/* -------------------------------------------------------------------------- */

const handlers: Record<string, Handler> = {
  check_app_update: () => ({ channel: null, version: null }),
  download_app_update: () => null,
  app_update_progress: () => 100,
  install_app_update: () => null,
  acknowledge_app_update_flush: () => null,
  /* --- accounts ------------------------------------------------------- */
  list_accounts: () => accounts.map(accountDto),
  // MX answers for the onboarding preselection; every other domain has none.
  detect_mail_provider: ({ domain }) => ({
    'acme-workspace.test': 'gmail',
    'contoso.test': 'outlook',
  } as Record<string, string>)[String(domain).toLowerCase()] ?? null,
  update_account_password: ({ accountId }) => {
    if (!accounts.some((account) => account.id === accountId)) throw new Error('backend.errors.accountNotFound');
    return undefined;
  },
  get_source_capabilities: () =>
    Object.fromEntries(
      accounts.map((account) => [
        account.id,
        {
          can_reply: true,
          can_forward: true,
          can_compose: true,
          has_folders: true,
          remote_flags: true,
          remote_archive: true,
          remote_delete: true,
          has_recipients: true,
          default_notify: true,
        },
      ]),
    ),

  /* --- threads -------------------------------------------------------- */
  list_threads: (payload) => {
    const scope = input(payload);
    const accountId = scope.account_id as string | undefined;
    const contact = String(scope.contact_email ?? '').toLowerCase();
    const mailbox = String(scope.mailbox_value ?? '');
    const rows = threads.filter(thread => {
      if (accountId && thread.accountId !== accountId) return false;
      if (scope.workflow_state && thread.workflowState !== scope.workflow_state) return false;
      if (contact && !thread.participants.some(person => person.email.toLowerCase() === contact)) return false;
      if (scope.smart_bucket && (scope.smart_bucket === 'pinned'
        ? !thread.isStarred : classifySmartBucket(thread) !== scope.smart_bucket)) return false;
      const folders = thread.mailboxNames ?? [];
      if (scope.mailbox_kind === 'folder' && !folders.includes(mailbox)) return false;
      if (scope.mailbox_kind === 'section') {
        if (mailbox === 'inbox' && (!['inbox', 'todo'].includes(thread.workflowState)
          || !folders.some(folder => folder.toLowerCase() === 'inbox'))) return false;
        const patterns: Record<string, RegExp> = {
          archive: /archive|archiv/i, sent: /sent/i, drafts: /draft/i,
          trash: /trash|deleted/i, spam: /spam|junk/i,
          notifications: /notif|updates|forum/i, blocked: /blocked/i,
        };
        const pattern = patterns[mailbox];
        if (pattern && !folders.some(folder => pattern.test(folder))) return false;
      }
      return true;
    }).sort((a, b) => Date.parse(b.sortDate ?? b.lastMessageAt) - Date.parse(a.sortDate ?? a.lastMessageAt)
      || b.id.localeCompare(a.id));
    const backwards = !!scope.before_cursor;
    const cursor = (scope.before_cursor ?? scope.cursor) as { id: string; last_message_date: string } | undefined;
    const date = cursor?.last_message_date;
    const eligible = cursor ? rows.filter(row => {
      const comparison = (row.sortDate ?? row.lastMessageAt).localeCompare(date!) || row.id.localeCompare(cursor.id);
      return backwards ? comparison > 0 : comparison < 0;
    }) : rows;
    if (backwards) eligible.reverse();
    const limit = Math.min(1000, Math.max(1, Math.trunc(Number(scope.limit ?? 200))));
    const page = eligible.slice(0, limit);
    const hasMore = eligible.length > limit;
    const last = page.at(-1);
    const nextCursor = hasMore && last ? { id: last.id, last_message_date: last.sortDate ?? last.lastMessageAt } : null;
    if (backwards) page.reverse();
    return { threads: page.map(threadDto), next_cursor: nextCursor, has_more: hasMore };
  },
  // No second-tier cache in the fixture: the fresh page is the only truth.
  list_cached_threads: () => null,
  get_threads: (payload) => [...new Set(payload.threadIds as string[])]
    .flatMap(id => { const thread = findThread(id); return thread ? [threadDto(thread)] : []; }),
  get_thread: (payload) => {
    const thread = findThread(payload.threadId as string);
    if (!thread) throw new Error('thread not found');
    return threadDto(thread);
  },
  // The fixture has no Message-ID header; one per thread is enough to link to.
  message_link_target: (payload) => {
    const thread = findThread(payload.threadId as string);
    if (!thread) throw new Error('thread not found');
    return { thread_id: thread.id, subject: thread.subject, message_id: `${thread.id}@mock.satchel.test` };
  },
  resolve_message_link: (payload) => {
    const match = /^<?(.+)@mock\.satchel\.test>?$/.exec(String(payload.messageId ?? '').trim());
    return match && findThread(match[1]!) ? match[1] : null;
  },
  list_smart_inbox_summary: (payload) => {
    const accountId = payload.accountId as string | null | undefined;
    const inbox = threads
      .filter((thread) => thread.workflowState === 'inbox' && (!accountId || thread.accountId === accountId))
      .sort(byDateDesc);
    const keys = ['priority', 'pinned', 'invitations', 'notifications', 'newsletter'] as const;
    const buckets = keys.flatMap((key) => {
      const rows = inbox.filter((thread) => classifySmartBucket(thread) === key);
      if (rows.length === 0) return [];
      return [{
        key,
        count: rows.length,
        unread_count: rows.reduce((sum, thread) => sum + thread.unreadCount, 0),
        preview_items: rows.slice(0, 4).map((thread) => ({
          id: thread.id,
          sender_name: thread.participants[0]?.name || thread.participants[0]?.email || '',
          sender_email: thread.participants[0]?.email ?? '',
          subject: thread.subject,
          unread_count: thread.unreadCount,
        })),
      }];
    });
    return { total_count: inbox.length, buckets };
  },
  update_thread_state: (payload) => {
    const scope = input(payload);
    const thread = findThread(scope.thread_id as string);
    if (!thread) throw new Error('thread not found');
    thread.workflowState = scope.next_state as WorkflowState;
    thread.snoozeUntil = (scope.snooze_until as string | null) ?? undefined;
    return threadDto(thread);
  },
  transition_threads: (payload) => {
    const scope = input(payload);
    const rows = (scope.threads as Array<{ thread_id: string }>) ?? [];
    const applied: string[] = [];
    for (const row of rows) {
      const thread = findThread(row.thread_id);
      if (!thread) continue;
      thread.workflowState = scope.to as WorkflowState;
      thread.snoozeUntil = (scope.snooze_until as string | null) ?? undefined;
      applied.push(thread.id);
    }
    return { applied, failed: [] };
  },
  set_threads_read: (payload) => {
    const isRead = input(payload).is_read as boolean;
    for (const id of ids(payload)) {
      const thread = findThread(id);
      if (thread) thread.unreadCount = isRead ? 0 : Math.max(1, thread.unreadCount);
      for (const message of messages[id] ?? []) message.isRead = isRead;
    }
  },
  set_threads_starred: (payload) => {
    const starred = input(payload).starred as boolean;
    for (const id of ids(payload)) {
      const thread = findThread(id);
      if (thread) thread.isStarred = starred;
    }
  },
  set_thread_category: (payload) => {
    const thread = findThread(payload.threadId as string);
    if (thread) thread.category = (payload.category ?? null) as Thread['category'];
  },
  set_thread_muted: (payload) => {
    const scope = input(payload);
    const thread = findThread(scope.thread_id as string);
    if (thread) thread.isMuted = scope.muted as boolean;
  },
  set_thread_priority: (payload) => {
    const scope = input(payload);
    const thread = findThread(scope.thread_id as string);
    if (thread) thread.priority = scope.priority as Thread['priority'];
  },
  set_thread_tags: (payload) => {
    const tags = (input(payload).tags as string[] | undefined) ?? [];
    for (const id of ids(payload)) {
      const thread = findThread(id);
      if (thread) thread.tags = [...tags];
    }
  },
  update_thread_organization: (payload) => {
    const scope = input(payload);
    const thread = findThread(scope.thread_id as string);
    if (!thread) return;
    thread.boardPosition = (scope.board_position as number | null) ?? undefined;
    thread.assignee = (scope.assignee as string | null) ?? undefined;
    thread.sharedWith = (scope.shared_with as string[] | undefined) ?? [];
  },
  trash_threads: (payload) => {
    for (const id of ids(payload)) {
      const index = threads.findIndex((thread) => thread.id === id);
      if (index >= 0) threads.splice(index, 1);
    }
  },
  mark_threads_spam: (payload) => {
    for (const id of ids(payload)) {
      const index = threads.findIndex((thread) => thread.id === id);
      if (index >= 0) threads.splice(index, 1);
    }
  },
  move_threads_to_folder: () => undefined,
  dismiss_mobile_notifications: () => undefined,
  get_sender_preference: (payload) => {
    const email = (payload.senderEmail as string) ?? '';
    const stored = senderPreferences.get(email);
    return {
      sender_email: email,
      category: (stored?.category ?? null) as null,
      is_priority: stored?.isPriority ?? false,
    };
  },
  set_sender_preference: (payload) => {
    const scope = input(payload);
    const email = scope.sender_email as string;
    const previous = senderPreferences.get(email) ?? { category: null, isPriority: false };
    senderPreferences.set(email, {
      category: (scope.category as string | null | undefined) ?? previous.category,
      isPriority: (scope.is_priority as boolean | undefined) ?? previous.isPriority,
    });
  },
  list_recent_transitions: () => [],
  run_rules_for_thread: () => [],
  run_rules_for_threads: () => [],

  /* --- messages ------------------------------------------------------- */
  list_messages_preview: (payload) =>
    (messages[payload.threadId as string] ?? []).map(messageDto),
  load_thread_messages: (payload) => {
    const scope = input(payload);
    const list = messages[scope.thread_id as string] ?? [];
    const count = Number(scope.body_count ?? 1);
    const latest = list.filter((message) => !message.isDraft).slice(-count);
    const drafts = list.filter((message) => message.isDraft);
    return {
      messages: list.map(messageDto),
      bodies: [...latest, ...drafts].map(messageBodyDto),
    };
  },
  list_message_bodies: (payload) => {
    const wanted = new Set((input(payload).message_ids as string[] | undefined) ?? []);
    return Object.values(messages)
      .flat()
      .filter((message) => wanted.has(message.id))
      .map(messageBodyDto);
  },
  load_message_body: (payload) => {
    const id = payload.messageId as string;
    const message = Object.values(messages).flat().find((entry) => entry.id === id);
    if (!message) throw new Error('message not found');
    return messageBodyDto(message);
  },
  /*
   * The seeded messages carry attachments, and returning none left the
   * attachment card — preview, open, save — unreachable in the mock, so
   * nothing exercised it and nobody saw it while building.
   */
  list_attachments_for_messages: (payload) => {
    const ids = new Set((payload.messageIds as string[] | undefined) ?? []);
    return Object.values(messages)
      .flat()
      .filter((message) => ids.has(message.id))
      .flatMap((message) => message.attachments.map((attachment) => ({
        id: attachment.id,
        message_id: message.id,
        filename: attachment.filename,
        content_type: attachment.contentType,
        size: attachment.size,
        content_id: null,
        is_inline: attachment.isInline ? 1 : 0,
        cached_path: null,
      })));
  },
  /*
   * A 2x2 PNG is enough: the card only has to receive bytes it can decode.
   * Anything else answers with its own name as text, which is what a plain
   * `.txt` or an unknown type would hold anyway.
   */
  download_attachment: (payload) => {
    const id = (input(payload).attachment_id as string | undefined) ?? '';
    const attachment = Object.values(messages)
      .flat()
      .flatMap((message) => message.attachments)
      .find((entry) => entry.id === id);
    const filename = attachment?.filename ?? 'attachment.bin';
    const isImage = attachment?.contentType.startsWith('image/') ?? false;
    return {
      filename,
      mime_type: attachment?.contentType ?? 'application/octet-stream',
      data_base64: isImage ? MOCK_PNG_BASE64 : btoa(filename),
      cached_path: null,
    };
  },
  get_message_source: (payload) => {
    const id = payload.messageId as string;
    const message = Object.values(messages).flat().find((entry) => entry.id === id);
    return {
      message_id: id,
      raw: `Subject: ${message?.subject ?? ''}\r\n\r\n${message?.bodyText ?? ''}`,
      truncated: false,
    };
  },
  search_message_hits: (payload) => {
    const request = input(payload);
    const query = String(request.query ?? '').trim().toLowerCase();
    const tokens = query.match(/(?:[^\s"]+|"[^"]*")+/g) ?? [];
    const notes: Array<{ operator: string; value: string }> = [];
    const filters = tokens.map((token) => {
      const colon = token.indexOf(':');
      const known = ['from', 'to', 'cc', 'subject', 'tag', 'in', 'is', 'has', 'before', 'after'];
      return colon < 0 || !known.includes(token.slice(0, colon))
        ? { operator: '', value: token.replaceAll('"', '') }
        : { operator: token.slice(0, colon), value: token.slice(colon + 1).replaceAll('"', '') };
    }).filter((token) => {
      if (!token.value) return false;
      let valid = true;
      if (token.operator === 'in') valid = ['inbox', 'todo', 'waiting', 'aside', 'done', 'snoozed', 'sent', 'drafts', 'trash', 'archive', 'spam'].includes(token.value);
      if (token.operator === 'is') valid = ['unread', 'starred', 'flagged'].includes(token.value);
      if (token.operator === 'has') valid = token.value === 'attachment';
      if (token.operator === 'before' || token.operator === 'after') {
        const relative = token.value.match(/^(\d+)([dwmy])$/);
        const milliseconds = relative ? Date.now() - Number(relative[1]) * (relative[2] === 'w' ? 7 : relative[2] === 'm' ? 30 : relative[2] === 'y' ? 365 : 1) * 86_400_000 : Date.parse(token.value);
        valid = Number.isFinite(milliseconds);
        if (valid) token.value = new Date(milliseconds).toISOString().slice(0, 10);
      }
      if (!valid) notes.push({ ...token });
      return valid;
    });
    const accountId = request.account_id;
    const cursor = request.cursor as { date: string; id: string } | null;
    const limit = Math.max(1, Math.min(500, Number(request.limit ?? 50)));
    const seen = new Set<string>();
    const hits = Object.values(messages).flat().filter((message) => {
      const thread = findThread(message.threadId);
      if (!query || !filters.some((token) => token.operator || !token.value.startsWith('-')) || !thread || (accountId && thread.accountId !== accountId)) return false;
      const text = `${message.subject} ${message.bodyText} ${message.from.name} ${message.from.email}`.toLowerCase();
      return filters.every(({ operator, value }) => {
        switch (operator) {
          case 'subject': return message.subject.toLowerCase().includes(value);
          case 'from': return `${message.from.name} ${message.from.email}`.toLowerCase().includes(value);
          case 'to': return message.to.some((person) => `${person.name} ${person.email}`.toLowerCase().includes(value));
          case 'cc': return message.cc?.some((person) => `${person.name} ${person.email}`.toLowerCase().includes(value)) ?? false;
          case 'is': return value === 'unread' ? !message.isRead : ['starred', 'flagged'].includes(value) ? thread.isStarred : false;
          case 'has': return value === 'attachment' && (thread.hasAttachments || message.attachments.length > 0);
          case 'in': return ['inbox', 'todo', 'waiting', 'aside', 'done', 'snoozed'].includes(value)
            ? value === thread.workflowState : thread.mailboxNames.some((name) => name.toLowerCase() === value);
          case 'tag': return thread.tags?.some((tag) => tag.toLowerCase().includes(value)) ?? false;
          case 'before': return message.timestamp < value;
          case 'after': return message.timestamp > value;
          default: return value.startsWith('-') ? !text.includes(value.slice(1)) : text.includes(value);
        }
      });
    }).sort((left, right) => right.timestamp.localeCompare(left.timestamp) || right.threadId.localeCompare(left.threadId) || right.id.localeCompare(left.id))
      .filter((message) => {
        if (seen.has(message.threadId)) return false;
        seen.add(message.threadId);
        return true;
      }).filter((message) => !cursor || message.timestamp < cursor.date || (message.timestamp === cursor.date && message.threadId < cursor.id))
      .slice(0, limit + 1)
      .map((message) => ({
        id: message.id,
        thread_id: message.threadId,
        subject: message.subject,
        snippet: message.bodyText.slice(0, 180),
        from_name: message.from.name,
        from_address: message.from.email,
        date: message.timestamp,
        is_read: message.isRead,
        workflow_state: findThread(message.threadId)?.workflowState ?? 'inbox',
        folder_type: '',
      }));
    const has_more = hits.length > limit;
    hits.length = Math.min(hits.length, limit);
    const last = hits.at(-1);
    return { hits, warnings: notes, has_more, next_cursor: has_more && last ? { rank_cost: 0, date: last.date, id: last.thread_id } : null };
  },

  /* --- outbox --------------------------------------------------------- */
  list_outbox: () => outbox.filter((item) => item.action_type === 'send_message').map((item) => ({ ...item })),
  list_failed_sync_actions: () => outbox.filter((item) => item.action_type === 'remote_mutation' && item.status === 'failed').map((item) => ({ ...item })),
  discard_failed_sync_action: (payload) => {
    const index = outbox.findIndex((item) => item.id === payload.id && item.action_type === 'remote_mutation' && item.status === 'failed');
    if (index < 0) throw new Error('backend.errors.outboxActionCannotBeDiscarded');
    outbox.splice(index, 1);
  },
  retry_outbox_action: (payload) => {
    const index = outbox.findIndex((item) => item.id === payload.id);
    if (index >= 0) outbox.splice(index, 1);
  },
  cancel_outbox_action: (payload) => {
    const index = outbox.findIndex((item) => item.id === payload.id);
    if (index < 0) throw new Error('outbox action not found');
    outbox.splice(index, 1);
    const draft = queuedDrafts.get(payload.id as string);
    if (draft) putDraft({ ...draft, scheduled_for: null });
    queuedDrafts.delete(payload.id as string);
  },
  retry_failed_sync_actions: () => 0,
  undo_outbox_send: (payload) => takeQueuedDraft(payload.id as string),

  /* --- drafts --------------------------------------------------------- */
  list_drafts: () => drafts.map((draft) => ({ ...draft })),
  load_draft: (payload) => {
    const queued = [...queuedDrafts].find(([, draft]) => draft.id === payload.id);
    if (queued) return { ...takeQueuedDraft(queued[0]), send_cancelled: true };
    return drafts.find((draft) => draft.id === payload.id) ?? null;
  },
  save_draft: (payload) => {
    const scope = input(payload);
    assertDraftRevision(scope.id, scope.expected_revision);
    assertDraftNotQueued(scope.id);
    const { expected_revision: _expected, ...fields } = scope;
    return putDraft(draftFromInput(fields));
  },
  // One window in the browser: every claim succeeds and nothing else holds a draft.
  claim_draft_editor: () => true,
  release_draft_editor: () => undefined,
  focus_draft_editor: () => false,
  delete_draft: (payload) => {
    for (const [id, draft] of queuedDrafts) {
      if (draft.id === payload.id) takeQueuedDraft(id);
    }
    const index = drafts.findIndex((draft) => draft.id === payload.id);
    if (index >= 0) drafts.splice(index, 1);
  },

  /* --- topics --------------------------------------------------------- */
  list_topics: () => topics.map((topic) => ({ ...topic })),
  get_topic_timeline: (payload) => topicTimeline[payload.topicId as string] ?? [],
  list_topic_memberships: () => [],
  add_topic_items: () => undefined,
  remove_topic_item: () => undefined,
  list_topic_files: () => [],

  /* --- composer ------------------------------------------------------- */
  send_message: (payload) => {
    const scope = input(payload);
    const accountId = payload.accountId as string;
    const recipients = (scope.to as string[] | undefined) ?? [];
    if (recipients.length === 0) throw new Error('missing recipient');

    assertDraftNotQueued(scope.draft_id);
    const scheduledFor = (scope.scheduled_for as string | null) ?? null;
    const messageId = nextId('msg');
    if (scheduledFor) {
      const queuedId = nextId('outbox');
      const draft = putDraft(draftFromInput({ ...scope, account_id: accountId, id: scope.draft_id ?? nextId('draft') }));
      queuedDrafts.set(queuedId, draft);
      outbox.unshift({
        id: queuedId,
        account_id: accountId,
        action_type: 'send_message',
        status: 'scheduled',
        retry_count: 0,
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        recipients,
        subject: (scope.subject as string) ?? '',
        last_error: null,
        next_retry_at: null,
        scheduled_for: scheduledFor,
      });
      return {
        status: 'scheduled',
        thread_id: (scope.thread_id as string) ?? '',
        message_id: messageId,
        queued_id: queuedId,
      };
    }

    const account = accounts.find((entry) => entry.id === accountId);
    const threadId = (scope.thread_id as string | null) || nextId('th');
    const now = new Date().toISOString();
    const sent: Message = {
      id: messageId,
      threadId,
      from: {
        name: (scope.from_name as string) || account?.displayName || '',
        email: (scope.from_address as string) || account?.email || '',
      },
      to: recipients.map((email) => ({ name: email, email })),
      subject: (scope.subject as string) ?? '',
      bodyText: (scope.body as string) ?? '',
      bodyHtml: (scope.body_html as string | null) ?? undefined,
      bodyLoaded: true,
      timestamp: now,
      isRead: true,
      attachments: [],
    };
    messages[threadId] = [...(messages[threadId] ?? []), sent];

    const thread = findThread(threadId);
    if (thread) {
      thread.lastMessageAt = now;
      thread.messageCount += 1;
      thread.snippet = sent.bodyText.slice(0, 180);
    } else {
      threads.unshift({
        id: threadId,
        accountId,
        subject: sent.subject,
        participants: [sent.from, ...sent.to],
        snippet: sent.bodyText.slice(0, 180),
        lastMessageAt: now,
        lastMessageAtMs: Date.parse(now),
        messageCount: 1,
        unreadCount: 0,
        hasAttachments: false,
        workflowState: 'done',
        priority: 1,
        mailboxNames: ['Sent'],
        tags: [],
      });
    }

    const draftId = scope.draft_id as string | null;
    if (draftId) {
      const index = drafts.findIndex((draft) => draft.id === draftId);
      if (index >= 0) drafts.splice(index, 1);
    }

    return { status: 'sent', thread_id: threadId, message_id: messageId, queued_id: null };
  },
  resolve_signature: () => null,
  list_signatures: () => [],
  list_signature_assignments: () => [],
  list_signature_suggestions: () => [],
  list_templates: () => [],
  list_aliases: () => [],
  list_smime_certificates: () => [],
  list_pgp_keys: () => [],
  pgp_engine_status: () => ({
    available: false, engine: 'sequoia', can_decrypt: false, can_sign: false, can_encrypt: false,
  }),
  credentials_status: () => ({
    total_accounts: accounts.length,
    encrypted_accounts: accounts.length,
    plaintext_accounts: 0,
    master_key_present: true,
    database_bytes: 0,
    restore_pending: false,
  }),

  /* --- odds and ends the bootstrap asks for --------------------------- */
  current_connection_state: () => ({
    online: true,
    syncing: false,
    last_sync_at: null,
    pending_outbox: outbox.length,
    pending_sends: outbox.filter((item) => item.action_type === 'send_message').length,
    failed_outbox: outbox.filter((item) => item.status === 'failed').length,
    failed_sends: outbox.filter((item) => item.action_type === 'send_message' && item.status === 'failed').length,
    failed_remote_mutations: outbox.filter((item) => item.action_type === 'remote_mutation' && item.status === 'failed').length,
  }),
  record_frontend_diagnostic: () => undefined,
  app_build_info: () => ({ version: '0.1.0', commit: 'mock', os: 'macos', arch: 'aarch64', debug: true }),
  open_logs_folder: () => undefined,
  diagnostics_overview: () => {
    const now = new Date().toISOString();
    const statuses = new Map<string, number>();
    for (const item of outbox) statuses.set(item.status, (statuses.get(item.status) ?? 0) + 1);
    return {
      generated_at: now,
      accounts: accounts.map((account) => ({
        id: account.id,
        email: account.email,
        display_name: account.displayName,
        provider: account.provider,
        sync_status: account.syncStatus ?? 'idle',
        last_sync_at: account.lastSyncAt ?? now,
        last_sync_error: account.lastSyncError ?? null,
        last_sync_error_at: account.lastSyncErrorAt ?? null,
        last_oauth_error: account.lastOauthError ?? null,
        last_oauth_error_at: account.lastOauthErrorAt ?? null,
      })),
      queue: {
        by_status: [...statuses].map(([status, entries]) => ({
          status, entries, total_retries: 0, max_retries: 0,
        })),
        recent_failures: outbox.filter((item) => item.status === 'failed').map((item) => ({
          id: item.id,
          account_id: item.account_id,
          action_type: item.action_type,
          status: item.status,
          error: item.last_error ?? null,
          attempts: item.retry_count,
          next_due: item.next_retry_at ?? null,
          updated_at: item.updated_at,
        })),
      },
      notifications: { by_status: [], recent: [] },
      notification_outbox: { by_state: [], reasons_24h: [], recent: [] },
      sync_leases: [],
      rate_limits: [],
    };
  },
  diagnostics_report: () => [
    'Satchel diagnostics (redacted)',
    'version: 0.1.0',
    'commit: mock',
    `generated: ${new Date().toISOString()}`,
  ].join('\n'),
  take_pending_notification_thread: () => null,
  consume_pending_mobile_notification: () => ({ threadId: null, actionId: null }),
  list_plugins: () => [],
  list_plugin_metrics: () => [],
  list_plugin_audit_log: () => [],
  list_mailboxes: () => [],
  list_smart_folders: () => [],
  list_follow_ups: () => [],
  list_invites: () => [],
  list_calendar_events: () => calendarEvents,
  list_calendar_sources: () => calendarSources,
  list_contacts: () => [],
  search_contacts: () => [],
  sync_local_contact_avatars: () => 0,
  list_blocked_senders: () => [],
  list_rules: () => [],
};

/* -------------------------------------------------------------------------- */
/* Transport                                                                  */
/* -------------------------------------------------------------------------- */

export async function sendCommand<T>(command: string, payload?: Payload): Promise<T> {
  try {
    assertCommandPayload(command, payload);
  } catch (error) {
    const message = `mock backend: invalid payload for "${command}": ${String(error)}`;
    console.error(message);
    throw new Error(message);
  }
  if (forcedFailures().has(command)) {
    throw new Error(`mock backend: forced failure for "${command}"`);
  }
  commandCounts[command] = (commandCounts[command] ?? 0) + 1;
  const handler = handlers[command];
  if (!handler) {
    // Loud on purpose: a silently swallowed command is how the suite ends up
    // proving nothing. `core-flows.pw.ts` fails the run on this message.
    const message = `mock backend: unimplemented command "${command}"`;
    console.error(message);
    throw new Error(message);
  }
  return handler(payload ?? {}) as T;
}
