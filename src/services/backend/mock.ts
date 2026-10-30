import { createPersistedSlot, persistence } from '@/persistence';
import type { BackendAccountVisibilityPolicy } from './accounts';
import { splitSubjectPrefixes } from '@/utils/composerText';
import { bytesToBase64 } from '@/utils/base64';
import { joinComposerTextHistory, splitComposerTextHistory, splitComposerHtmlHistory } from '@/utils/composerHistory';
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
import type { BackendContact } from '@/types/contacts';
import type { BackendDraft, BackendSignature, BackendResolvedSignature } from '@/types/composer';
import type { BackendMessage, BackendMessageBody } from '@/types/messages';
import type { BackendCalendarEvent, BackendCalendarSource } from '@/types/calendar';
import type { BackendOutboxItem } from '@/types/outbox';
import type { BackendTopic, BackendTopicTimelineItem } from '@/types/topics';
import { matchesWorkflowFilter, type WorkflowFilter, type WorkflowState } from '@/types/workflow';
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
const visibilitySlot = createPersistedSlot<Record<string, BackendAccountVisibilityPolicy>>('satchel.mock.accountVisibility', {
  fallback: () => ({}),
  decode: raw => JSON.parse(raw),
  encode: value => JSON.stringify(value),
});
const visibilityPolicies = persistence.read(visibilitySlot);
function defaultVisibilityPolicy(): BackendAccountVisibilityPolicy {
  return { mode: 'always', weekdays: [1, 2, 3, 4, 5], from: null, to: null, override_date: null, override_visible: null };
}
function localDate(now: Date): string {
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
}
function accountVisible(accountId: string, now = new Date()): boolean {
  const policy = visibilityPolicies[accountId] ?? defaultVisibilityPolicy();
  if (policy.override_date === localDate(now) && policy.override_visible !== null) return policy.override_visible;
  if (policy.mode === 'always') return true;
  const time = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`;
  return policy.mode === 'schedule' && policy.weekdays.includes(now.getDay())
    && time >= (policy.from ?? '00:00') && time < (policy.to ?? '24:00');
}
// 96px PNG, a rowing boat at sunset: an account picture the user chose.
const MOCK_CLUB_AVATAR = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAGAAAABgCAMAAADVRocKAAAASFBMVEX/8Mj+w278wm/7wW/6wHD5v3D4v3H3vnH1vXL0vHPzu3PyunTwuXXvuHXut3bttnbstnfrtXfqtHjos3nnsnlGgrQmXIwoHh7S40JTAAABZklEQVR42u2T21LDMAwFxb2QFGhx7P//Ux6apmAf2YosM03xvmlG3m3SmO4aQz1QDNw3pgfKgYfGtA88NuYGAk+NaR94bkxtgE5kFl5qoAvsio2eT5CZnynQTgulwDVDPyxcbYBIWKBXHUwALJr6QYHeVLCBdHP7gUEFG0g3/2tgkPoHGnUwAbA4WhbGLQVGmX+kvZrEj7f2ZgVm6b2GH3p2JSsIIZTmvD4fCCfEMw588MwC8Qyhon8xlGYm8MmyCIQzhg4si0A4YzKBw+V8EM36wPwrS7M6cD5fmnHgyLO85CCbIaKAcFYHpDMOfPHM58Uz5AYCjmcWiGcIAWM9XMDZ+6OAM/fHAWftd+Qjqv2RLwl4Wz8IeFM/CnhL/zkw/cLQbxvwfMDkFXlhIF4F81Qk/5nW+zMBgd+vgRr7wROsnIWB8rvU8leBdigC01T1mV5BYPv/Qb8H/R70e9DvQb8H/R6s5xsUyBEGcUIQNgAAAABJRU5ErkJggg==';
// A second mailbox for visibility walkthroughs; all policy behavior still runs
// through the same command handlers as the ordinary fixture.
if (typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('mockVisibility')) {
  // Three mailboxes in three colours, one of them with a picture of its own.
  accounts.push(
    { ...accounts[0]!, id: 'acc-personal', displayName: 'Personal', email: 'tom@example.net', color: '#4cd68a' },
    { ...accounts[0]!, id: 'acc-club', displayName: 'Rowing club', email: 'kasse@ruderclub.example', color: '#f0b232', avatar: MOCK_CLUB_AVATAR },
  );
  const extra: Array<[string, string, string]> = [
    ['acc-personal', 'Sunday lunch at our place', 'Alex'],
    ['acc-personal', 'Your weekend reading', 'Alex'],
    ['acc-personal', 'Photos from the coast', 'Alex'],
    ['acc-club', 'Boat house keys for Saturday', 'Mia'],
    ['acc-club', 'Membership fees 2027', 'Mia'],
  ];
  for (const [index, [accountId, subject, sender]] of extra.entries()) {
    const template = threads.find(thread => thread.id === 'th-1')!;
    const id = `th-${accountId.slice(4)}-${index}`;
    const owner = accounts.find(account => account.id === accountId)!;
    const from = { name: sender, email: `${sender.toLowerCase()}@example.net` };
    threads.push({ ...template, id, accountId, subject, priority: 0, messageCount: 1, senderKnown: true, hasAttachments: false,
      snippet: 'A quiet weekend, with time to catch up.', tags: [], dueDate: undefined,
      participants: [from, { name: 'Tom', email: owner.email }],
      lastMessageAt: new Date(Date.now() - (index + 1) * 3600_000).toISOString(),
      lastMessageAtMs: Date.now() - (index + 1) * 3600_000,
    });
    messages[id] = [{ ...messages['th-1']![0]!, id: `${id}-m`, threadId: id, subject,
      from, to: [{ name: 'Tom', email: owner.email }],
      attachments: [],
      bodyText: `Hi Tom, a quiet weekend, with time to catch up. See you on Sunday! ${sender}`,
      bodyHtml: `<p>Hi Tom,</p><p>A quiet weekend, with time to catch up.</p><p>See you on Sunday!<br>${sender}</p>`,
    }];
  }
  visibilityPolicies['acc-work'] ??= { ...defaultVisibilityPolicy(), mode: 'schedule', weekdays: [1, 2, 4, 5], from: '09:00', to: '17:00' };
}
const trashReceipts = new Map<string, Thread[]>();
let nextTrashReceipt = 0;
const outbox: BackendOutboxItem[] = mockOutboxItems.map((item) => ({
  ...item,
  last_error: item.id === 'failed-move' ? 'backend.errors.queueFolderChanged'
    : item.id === 'ob-failed' ? 'backend.errors.queueMessageTooLarge' : item.last_error,
}));
// `?mockOutbox=0` starts with a clean Outbox, for screens that are not about it.
if (typeof window !== 'undefined' && (new URLSearchParams(window.location.search).has('mockVisibility')
  || new URLSearchParams(window.location.search).get('mockOutbox') === '0')) outbox.splice(0);
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

// Opt-in composer fixtures also exercise signature switching and plain-text mode.
const signatures: BackendSignature[] = searchNumber('mockSignatures', 1) ? accounts
  .filter((account) => account.kind === 'email')
  .flatMap((account) => ['Default', 'Brief'].map((name, index) => ({
    id: `signature-${account.id}-${index}`,
    account_id: account.id,
    name,
    body_text: index ? `Thanks,\n${account.displayName}` : `Kind regards,\n${account.displayName}\n${account.email}`,
    body_html: index ? `<p>Thanks,<br>${account.displayName}</p>` : `<p>Kind regards,<br><strong>${account.displayName}</strong><br>${account.email}</p>`,
    short_body_text: `Thanks,\n${account.displayName}`,
    short_body_html: `<p>Thanks,<br>${account.displayName}</p>`,
    assets: [],
    is_default: index === 0 ? 1 : 0,
    created_at: '',
    updated_at: '',
  }))) : [];

function resolveMockSignature(scope: Payload): BackendResolvedSignature | null {
  if (scope.preference === 'none' || scope.signature_variant === 'none') return null;
  const selected = signatures.find((signature) => signature.id === scope.signature_id);
  if (!selected) return null;
  const variant = scope.preference === 'short' || scope.signature_variant === 'short' ? 'short' : 'long';
  return {
    signature_id: selected.id,
    variant,
    body_text: variant === 'short' ? selected.short_body_text : selected.body_text,
    body_html: variant === 'short' ? selected.short_body_html ?? null : selected.body_html ?? null,
    attachments: [],
  };
}

function mockSignedBody(scope: Payload) {
  const resolved = resolveMockSignature(scope);
  const body = (scope.body as string) ?? '';
  const bodyHtml = (scope.body_html as string | null) ?? null;
  if (!resolved) return { body, bodyHtml };
  const text = (scope.signature_override_text as string | null) ?? resolved.body_text;
  const html = (scope.signature_override_html as string | null) ?? resolved.body_html;
  const parts = splitComposerTextHistory(body);
  const signed = text.trim() ? `${parts.writing.trimEnd()}\n\n-- \n${text}` : parts.writing;
  const htmlParts = bodyHtml === null ? null : splitComposerHtmlHistory(bodyHtml);
  return {
    body: joinComposerTextHistory(signed, parts.history),
    bodyHtml: htmlParts ? `${htmlParts.writing}<div data-satchel-signature="${resolved.signature_id}">${html ?? text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\n/g, '<br>')}</div>${htmlParts.history}` : null,
  };
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

// Ingest touches senders and To/Cc recipients in the same contact table.
const contacts: BackendContact[] = [];
for (const message of Object.values(messages).flat().filter((message) => !message.isDraft)) {
  const account = accounts.find((item) => item.email.trim().toLowerCase() === message.from.email.trim().toLowerCase());
  const sender = { ...message.from, name: account?.displayName || message.from.name };
  for (const person of [sender, ...[...message.to, ...(message.cc ?? [])].map((recipient) => ({ ...recipient, name: '' }))]) {
    const email = person.email.trim().toLowerCase();
    if (!email.includes('@')) continue;
    const existing = contacts.find((contact) => contact.email === email);
    if (existing) {
      existing.frequency += 1;
      if (person.name) existing.display_name = person.name;
      if (!existing.last_contacted || message.timestamp > existing.last_contacted) existing.last_contacted = message.timestamp;
    } else {
      contacts.push({
        id: `contact-${email}`, email, display_name: person.name || null,
        avatar_url: null, avatar_source: null, avatar_locked: 0, avatar_updated_at: null,
        frequency: 1, last_contacted: message.timestamp, notes: null,
        created_at: message.timestamp, updated_at: message.timestamp,
      });
    }
  }
}
function sortedContacts() {
  return [...contacts].sort((left, right) => right.frequency - left.frequency
    || (right.last_contacted ?? right.created_at).localeCompare(left.last_contacted ?? left.created_at)
    || left.email.localeCompare(right.email));
}

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
const topicTimeline: Record<string, BackendTopicTimelineItem[]> = Object.fromEntries(Object.entries(mockTopicTimeline).map(([id, rows]) => [id, rows.map((row) => ({ ...row }))]));
const topicLinks: { topic_id: string; item_type: string; item_id: string }[] = Object.entries(topicTimeline)
  .flatMap(([topicId, rows]) => rows.filter((row) => row.item_type !== 'note').map((row) => ({
    topic_id: topicId, item_type: row.item_type, item_id: row.item_id,
  })));
const topicVersions = new Map<string, string>();
const removedTopicItems = new Map<string, BackendTopicTimelineItem>();
function topicLinkKey(topicId: string, itemType: string, itemId: string): string {
  return JSON.stringify([topicId, itemType, itemId]);
}

function changeTopicLink(topicId: string, itemType: string, itemId: string, present: boolean, addedBy = 'user', restore = false) {
  const topic = topics.find((topic) => topic.id === topicId);
  if (!topic) throw new Error('topic not found');
  const index = topicLinks.findIndex((link) => link.topic_id === topicId && link.item_type === itemType && link.item_id === itemId);
  if ((index >= 0) === present) return null;
  const key = topicLinkKey(topicId, itemType, itemId);
  const rows = topicTimeline[topicId] ?? [];
  if (present) {
    topicLinks.push({ topic_id: topicId, item_type: itemType, item_id: itemId });
    // Events are memberships but are rendered separately, like Core's timeline.
    if (itemType === 'thread' || itemType === 'message') {
      const message = itemType === 'message' ? Object.values(messages).flat().find((m) => m.id === itemId) : undefined;
      const thread = findThread(message?.threadId ?? itemId);
      if (thread || message) {
        const now = new Date().toISOString();
        const original = restore ? removedTopicItems.get(key) : undefined;
        rows.push(original ?? {
          item_type: itemType, item_id: itemId, thread_id: message?.threadId ?? itemId,
          title: message?.subject ?? thread!.subject, snippet: message?.bodyText ?? thread!.snippet,
          content: null, from_address: message?.from.email ?? thread?.participants[0]?.email ?? null,
          sort_date: message?.timestamp ?? thread!.lastMessageAt, added_at: now, added_by: addedBy,
          message_count: itemType === 'message' ? 1 : thread!.messageCount,
          unread_count: message ? Number(!message.isRead) : thread!.unreadCount,
        });
      }
    }
    topic.item_count++;
  } else {
    topicLinks.splice(index, 1);
    const row = rows.find((row) => row.item_type === itemType && row.item_id === itemId);
    if (row) removedTopicItems.set(key, row);
    topic.item_count = Math.max(0, topic.item_count - 1);
  }
  topicTimeline[topicId] = (present ? rows : rows.filter((row) => row.item_type !== itemType || row.item_id !== itemId))
    .sort((a, b) => b.sort_date.localeCompare(a.sort_date));
  const revision = crypto.randomUUID();
  topicVersions.set(key, revision);
  return { item_id: itemId, revision };
}

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

/**
 * A draft that belongs to a thread is also a message in it, as in the
 * database. Deleting the draft takes the message out of the thread; saving it
 * again (an undo) puts it back where it was.
 */
const removedDraftMessages = new Map<string, Message>();

function syncDraftMessage(draft: BackendDraft) {
  const threadId = draft.thread_id;
  if (!threadId) return;
  const list = messages[threadId] ?? [];
  const current = list.find((message) => message.id === draft.id) ?? removedDraftMessages.get(draft.id);
  if (!current) return;
  removedDraftMessages.delete(draft.id);
  const updated = { ...current, bodyText: draft.body, timestamp: draft.updated_at };
  messages[threadId] = [...list.filter((message) => message.id !== draft.id), updated];
}

function removeDraftMessage(id: string) {
  for (const [threadId, list] of Object.entries(messages)) {
    const message = list.find((entry) => entry.id === id && entry.isDraft);
    if (!message) continue;
    removedDraftMessages.set(id, message);
    messages[threadId] = list.filter((entry) => entry !== message);
  }
}

function putDraft(draft: BackendDraft) {
  const index = drafts.findIndex((entry) => entry.id === draft.id);
  const revision = (index >= 0 ? drafts[index]!.revision ?? 0 : 0) + 1;
  const stored = { ...draft, revision };
  if (index >= 0) drafts.splice(index, 1, stored);
  else drafts.unshift(stored);
  syncDraftMessage(stored);
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

/**
 * A 336x208 sketch of a login page under an error banner, about 800 bytes.
 * Every image attachment in the demo mailbox thumbnails from it, so it has to
 * read as a screenshot; a flat swatch made the card look broken.
 */
const MOCK_PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAAVAAAADQBAMAAACjASSGAAAAKlBMVEX////09vjs7/P94uDe4ufc4efW2+H0v0+qsbphxVQX' +
  'cH/sal7WRUE0OkLa/vY3AAACuElEQVR42u2dMU5bQRCGx46xiCUk06PIMhfgBMgFB6AJDc2rnDYlJUegdvUaGtJwAAofIQdI' +
  'kE8AlixZkbHsFCslSCD8dmc9b5/ft9Ir8DDaj39mdnYXya/xVaoxmgIooIBGGa2Nv7EcW3CcFVS0PXQ/9R39azcTzs2zONB2' +
  'JkMRafakLyKtwas/cGkT2aU6R8dGOTguBJq5pyciPZGBuIeqrwFo7p6JiExcuowrp6hVsg4KgS5yGYnIaiKPbol/8OgINo2n' +
  'sXE/mkhn2ixY64yqBxTQXQJt9FEUUEABBRRQQLdxBHin63b10z4bKBqDUw7J0f+jG2XeLopWsOpF5PPNx/ZvqSh6o7TTmSoL' +
  '+l1pNzmFHkaZdz0l9JUDnUaZd4qi4WUQZz8a0ELXzxQToIACmugOX3udvprYgPYftOqc+pMG/LOh+Usfxy8WOXqkT7hlz0DR' +
  'CIKGSLrDy9NRjGn9Y8+CDyiggO7S7smN3x/YZifpKHoQbDQG/amwUkyVBj1RWE1BZ8FG6+XpmBwFFFBAAQUUUEAT2JTsX4d4' +
  'Xdkrem3oRY4mBWoc+vBi+nNF6Gu6jko78/cZlQDavvzh7zQclRD6eyMfLehliNNLRtUDCiig2wa9DXHay0tQ9NzIR9tCF7cV' +
  '6fWyGFFMgAIKaIpnJu6eyNHUQY1Dz90T66j/4O4pdujvjXy0oNw9AQoooImCcvcUu4Vy90TVAwoooIACCmgdQRWbkk7A3vKi' +
  'DEVzIx8taMfMSQmaG3pR9WlV/Zsxf3OIuktT0azAJ3SmnQTNI67vWy2mzh2hrwVoZuilAp2bOWlDn1kKqqn6+QXFBCiggAIK' +
  'KKCA1g70k/dXh8+eYhzV1oT+31gNIkx7ahD6GLH3j3xI6FfqlzO2/AUNe31cGd+Ry3vuAAUUUEABBRRQQAEFtMD4C4tNcaDi' +
  'eAAwAAAAAElFTkSuQmCC';

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

function threadDrafts(threadId: string): Message[] {
  return (messages[threadId] ?? []).filter((message) => message.isDraft);
}

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
    // As the core reads them: from the thread's unsent draft messages, so a
    // draft saved, resumed or discarded here shows in the row like it would.
    has_draft: Boolean(thread.hasDraft) || threadDrafts(thread.id).length > 0,
    draft_snippet: thread.draftSnippet ?? (threadDrafts(thread.id).at(-1)?.bodyText.slice(0, 180) || null),
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
    workflow_changed_at: thread.workflowChangedAt ?? null,
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
    avatar: account.avatar ?? null,
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
    // Like the backend preview: the unsubscribe headers travel with the body.
    list_unsubscribe: null,
    list_unsubscribe_post: null,
    auth_spf: message.authSpf ?? null,
    auth_dkim: message.authDkim ?? null,
    auth_dmarc: message.authDmarc ?? null,
  };
}

function hasCachedBody(message: Message): boolean {
  return Boolean(message.bodyText.trim() || message.bodyHtml?.trim());
}

function messageBodyDto(message: Message): BackendMessageBody {
  return {
    id: message.id,
    snippet: message.bodyText.slice(0, 180),
    body_text: message.bodyText,
    body_html: message.bodyHtml ?? null,
    body_loaded: hasCachedBody(message),
    list_unsubscribe: message.listUnsubscribe ?? null,
    list_unsubscribe_post: message.listUnsubscribePost ?? null,
    auth_spf: message.authSpf ?? null,
    auth_dkim: message.authDkim ?? null,
    auth_dmarc: message.authDmarc ?? null,
  };
}

/** Provider fixtures retain their bodies independently of the local cache. */
const serverMessages: Array<{ accountId: string; remoteId: string; message: Message; thread: Thread }> = [];
if (typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('mockServerSearch')) {
  const baseThread = threads[0]!;
  const baseMessage = messages[baseThread.id]![0]!;
  accounts[0]!.provider = 'gmail';
  accounts.push({ ...accounts[0]!, id: 'acc-server-other', email: 'other@example.com', provider: 'outlook' });
  for (const accountId of [accounts[0]!.id, 'acc-server-other']) {
    for (const retained of [true, false]) {
      const thread = { ...baseThread, id: `th-server-${accountId}-${retained}`, accountId };
      const message = {
        ...baseMessage, id: `m-server-${accountId}-${retained}`, threadId: thread.id,
        subject: 'Provider fixture', bodyText: 'Server search needle', bodyHtml: undefined,
      };
      serverMessages.push({ accountId, remoteId: retained ? 'shared-remote' : 'server-only', message, thread });
      if (retained) {
        threads.push({ ...thread });
        messages[thread.id] = [{ ...message, bodyText: '' }];
      }
    }
  }
}

function restoreServerBody(message: Message): boolean {
  if (hasCachedBody(message)) return false;
  const remote = serverMessages.find(entry => entry.message.id === message.id);
  if (!remote) throw new Error('message body not available remotely');
  message.bodyText = remote.message.bodyText;
  message.bodyHtml = remote.message.bodyHtml;
  return true;
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

/**
 * Whether a thread is in the list a request names — the contact, workflow
 * state and mailbox of `list_threads`, without account or bucket. The same
 * test narrows `search_message_hits` to a list, as `ThreadListScope` does.
 */
function inThreadList(thread: Thread, scope: Record<string, unknown>): boolean {
  const contact = String(scope.contact_email ?? '').toLowerCase();
  const mailbox = String(scope.mailbox_value ?? '');
  if (scope.workflow_state && !matchesWorkflowFilter(thread.workflowState, scope.workflow_state as WorkflowFilter)) return false;
  if (contact && !thread.participants.some(person => person.email.toLowerCase() === contact)) return false;
  const folders = thread.mailboxNames ?? [];
  if (scope.mailbox_kind === 'folder' && mailbox && !folders.includes(mailbox)) return false;
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
}

function byDateDesc(a: Thread, b: Thread): number {
  return new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime();
}

/* -------------------------------------------------------------------------- */
/* Command table                                                              */
/* -------------------------------------------------------------------------- */

const googleMailScope = 'https://www.googleapis.com/auth/gmail.modify';
const oauthScopes = new Map<string, string[]>();
const oauthFlows = new Map<string, { accountId: string; scopes: string[] }>();
function oauthStatus(accountId: string) {
  const account = accounts.find((item) => item.id === accountId);
  return { account_id: accountId, provider: account?.provider ?? 'gmail', connected: true,
    has_refresh_token: true, expires_at: null, granted_scopes: oauthScopes.get(accountId) ?? [googleMailScope] };
}

let appUpdateChannel: 'stable' | 'nightly' = 'stable';

const handlers: Record<string, Handler> = {
  oauth_status: (payload) => oauthStatus(String(payload.accountId)),
  start_oauth_flow: (payload) => {
    const input = payload.input as { account_id: string; purpose?: string | null };
    const scopes = [...(oauthScopes.get(input.account_id) ?? [googleMailScope])];
    const optional = input.purpose === 'calendar' ? 'https://www.googleapis.com/auth/calendar.readonly'
      : input.purpose === 'contacts' ? 'https://www.googleapis.com/auth/contacts.readonly' : null;
    if (optional && !scopes.includes(optional)) scopes.push(optional);
    const state = `flow-${oauthFlows.size + 1}`;
    oauthFlows.set(state, { accountId: input.account_id, scopes });
    return { state, authorization_url: 'https://accounts.google.com/o/oauth2/v2/auth',
      expires_at: new Date(Date.now() + 900_000).toISOString(), browser_opened: true, browser_error: null };
  },
  poll_oauth_flow: (payload) => {
    const input = payload.input as { state: string };
    const flow = oauthFlows.get(input.state);
    if (!flow) return { status: 'failed', result: null, error: 'backend.errors.oauthFlowNotFound' };
    oauthScopes.set(flow.accountId, flow.scopes);
    return { status: 'completed', result: oauthStatus(flow.accountId), error: null };
  },
  sync_google_calendar: () => ({ accounts_checked: accounts.filter((account) => account.provider === 'gmail').length,
    events_synced: 0, errors: [] }),
  // No mock account carries a CalDAV/CardDAV service, so the real command
  // would skip every one of them and report an empty pass.
  sync_groupware: () => ({
    accounts_checked: 0,
    calendars_found: 0,
    events_synced: 0,
    address_books_found: 0,
    contacts_synced: 0,
    errors: [],
  }),
  get_app_update_preferences: () => ({ channel: appUpdateChannel, supported: true }),
  set_app_update_channel: (payload) => { appUpdateChannel = payload.channel as 'stable' | 'nightly'; },
  check_app_update: () => ({ channel: appUpdateChannel, version: null }),
  download_app_update: () => null,
  app_update_progress: () => 100,
  install_app_update: () => null,
  acknowledge_app_update_flush: () => null,
  /* --- accounts ------------------------------------------------------- */
  list_accounts: () => accounts.map(accountDto),
  get_account_visibility: () => accounts.map(account => ({
    account_id: account.id, policy: visibilityPolicies[account.id] ?? defaultVisibilityPolicy(),
    visible: accountVisible(account.id), local_date: localDate(new Date()),
  })),
  // The same refusals as AccountRepository::set_color / set_avatar.
  set_account_color: ({ accountId, color }) => {
    const account = accounts.find((item) => item.id === accountId);
    if (!account) throw new Error('backend.errors.accountNotFound');
    if (!/^#[0-9a-f]{6}$/i.test(String(color))) throw new Error('backend.errors.accountColorInvalid');
    account.color = String(color).toLowerCase();
  },
  set_account_avatar: ({ accountId, avatar }) => {
    const account = accounts.find((item) => item.id === accountId);
    if (!account) throw new Error('backend.errors.accountNotFound');
    if (avatar != null && (!/^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/]+=*$/.test(String(avatar))
      || String(avatar).length > 96 * 1024)) throw new Error('backend.errors.accountAvatarInvalid');
    account.avatar = (avatar as string | null | undefined) ?? null;
  },
  set_account_visibility: (payload) => {
    visibilityPolicies[payload.accountId as string] = JSON.parse(JSON.stringify(payload.policy)) as BackendAccountVisibilityPolicy;
    persistence.write(visibilitySlot, visibilityPolicies);
  },
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
    const rows = threads.filter(thread => {
      if (accountId ? thread.accountId !== accountId : !accountVisible(thread.accountId)) return false;
      if (scope.smart_bucket && (scope.smart_bucket === 'pinned'
        ? !thread.isStarred : classifySmartBucket(thread) !== scope.smart_bucket)) return false;
      return inThreadList(thread, scope);
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
    if (!thread) throw new Error('backend.errors.threadNotFound');
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
      .filter((thread) => thread.workflowState === 'inbox' && (accountId ? thread.accountId === accountId : accountVisible(thread.accountId)))
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
  transition_threads: (payload) => {
    const scope = input(payload);
    const rows = (scope.threads as Array<{ thread_id: string }>) ?? [];
    const applied: string[] = [];
    for (const row of rows) {
      const thread = findThread(row.thread_id);
      if (!thread) continue;
      thread.workflowState = scope.to as WorkflowState;
      thread.workflowChangedAt = new Date().toISOString();
      thread.snoozeUntil = (scope.snooze_until as string | null) ?? undefined;
      // Done includes Read, committed with the transition like the backend does.
      if (scope.to === 'done') {
        thread.unreadCount = 0;
        for (const message of messages[thread.id] ?? []) message.isRead = true;
      }
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
    const removed: Thread[] = [];
    for (const id of ids(payload)) {
      const index = threads.findIndex((thread) => thread.id === id);
      if (index >= 0) removed.push(...threads.splice(index, 1));
    }
    const token = `trash-${++nextTrashReceipt}`;
    trashReceipts.set(token, removed);
    return { undo_token: token, undo_thread_ids: removed.map((thread) => thread.id), permanently_deleted: [] };
  },
  undo_trash: (payload) => {
    const token = payload.receiptId as string;
    const restored = trashReceipts.get(token) ?? [];
    for (const thread of restored) if (!findThread(thread.id)) threads.push(thread);
    trashReceipts.delete(token);
  },
  mark_threads_spam: (payload) => {
    const removed: Thread[] = [];
    for (const id of ids(payload)) {
      const index = threads.findIndex((thread) => thread.id === id);
      if (index >= 0) removed.push(...threads.splice(index, 1));
    }
    // Same receipt shape and Undo path as the backend, which reuses trash receipts.
    const token = `spam-${++nextTrashReceipt}`;
    trashReceipts.set(token, removed);
    return { undo_token: token, undo_thread_ids: removed.map((thread) => thread.id), permanently_deleted: [] };
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
    const id = input(payload).message_id as string;
    const message = Object.values(messages).flat().find((entry) => entry.id === id);
    if (!message) throw new Error('message not found');
    const fetched = restoreServerBody(message);
    return { body_text: message.bodyText, body_html: message.bodyHtml ?? null, fetched };
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
  export_message: () => null,
  get_message_source: (payload) => {
    const id = payload.messageId as string;
    const message = Object.values(messages).flat().find((entry) => entry.id === id);
    return {
      message_id: id,
      raw: `Subject: ${message?.subject ?? ''}\r\n\r\n${message?.bodyText ?? ''}`,
      truncated: false,
    };
  },
  thread_eml_attachments: (payload) => [...new Set(payload.threadIds as string[])].flatMap((threadId) => {
    const message = (messages[threadId] ?? [])
      .filter((entry) => !entry.isDraft)
      .sort((a, b) => b.timestamp.localeCompare(a.timestamp))[0];
    if (!message) return [];
    const raw = [
      `From: ${message.from.name} <${message.from.email}>`,
      `To: ${message.to.map((participant) => participant.email).join(', ')}`,
      `Subject: ${message.subject}`,
      `Date: ${new Date(message.timestamp).toUTCString()}`,
      'Content-Type: text/plain; charset=utf-8',
      '',
      message.bodyText,
    ].join('\r\n');
    return [{
      filename: `${message.subject.replace(/[\\/:*?"<>|]/g, '_').trim() || 'message'}.eml`,
      mime_type: 'message/rfc822',
      content_base64: bytesToBase64(new TextEncoder().encode(raw)),
      content_id: null,
      inline: false,
    }];
  }),
  server_search: (payload) => {
    const request = input(payload);
    const accountId = String(request.account_id);
    const account = accounts.find(account => account.id === accountId);
    if (!account) throw new Error('account not found');
    if (!['gmail', 'outlook'].includes(account.provider)) return [];
    if (typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('mockServerSearchFailure') === accountId) {
      throw new Error('network is unreachable');
    }
    const query = String(request.query ?? '').trim().toLowerCase();
    const local = handlers.search_message_hits!({ input: { query, account_id: accountId, limit: 500 } }) as { hits: Array<{ id: string }> };
    const known = new Set(local.hits.map(hit => hit.id));
    return serverMessages.filter(entry => entry.accountId === accountId
      && query && `${entry.message.subject} ${entry.message.bodyText}`.toLowerCase().includes(query)
      && !known.has(entry.message.id)).map(({ remoteId, message }) => ({
      remote_id: remoteId, subject: message.subject, snippet: message.bodyText,
      from: message.from.email, date: message.timestamp,
    }));
  },
  fetch_server_search_hit: (payload) => {
    const request = input(payload);
    const entry = serverMessages.find(entry => entry.accountId === request.account_id && entry.remoteId === request.remote_id);
    if (!entry) throw new Error('message not found on server');
    const local = messages[entry.thread.id]?.find(message => message.id === entry.message.id);
    if (local) {
      restoreServerBody(local);
    } else {
      threads.push({ ...entry.thread });
      messages[entry.thread.id] = [{ ...entry.message }];
    }
    return { message_id: entry.message.id, thread_id: entry.thread.id };
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
      if (token.operator === 'in') valid = ['inbox', 'todo', 'waiting', 'aside', 'done', 'snoozed', 'later', 'sent', 'drafts', 'trash', 'archive', 'spam'].includes(token.value);
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
    const listScope = (request.list_scope ?? null) as Record<string, unknown> | null;
    const cursor = request.cursor as { date: string; id: string } | null;
    const limit = Math.max(1, Math.min(500, Number(request.limit ?? 50)));
    const seen = new Set<string>();
    const hits = Object.values(messages).flat().filter((message) => {
      const thread = findThread(message.threadId);
      if (!query || !filters.some((token) => token.operator || !token.value.startsWith('-')) || !thread || (accountId ? thread.accountId !== accountId : !accountVisible(thread.accountId))) return false;
      if (listScope && !inThreadList(thread, listScope)) return false;
      const text = `${message.subject} ${message.bodyText} ${message.from.name} ${message.from.email}`.toLowerCase();
      return filters.every(({ operator, value }) => {
        switch (operator) {
          case 'subject': return message.subject.toLowerCase().includes(value);
          case 'from': return `${message.from.name} ${message.from.email}`.toLowerCase().includes(value);
          case 'to': return message.to.some((person) => `${person.name} ${person.email}`.toLowerCase().includes(value));
          case 'cc': return message.cc?.some((person) => `${person.name} ${person.email}`.toLowerCase().includes(value)) ?? false;
          case 'is': return value === 'unread' ? !message.isRead : ['starred', 'flagged'].includes(value) ? thread.isStarred : false;
          case 'has': return value === 'attachment' && (thread.hasAttachments || message.attachments.length > 0);
          case 'in': return ['inbox', 'todo', 'waiting', 'aside', 'done', 'snoozed', 'later'].includes(value)
            ? matchesWorkflowFilter(thread.workflowState, value as WorkflowFilter) : thread.mailboxNames.some((name) => name.toLowerCase() === value);
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
  retry_failed_sync_actions: () => {
    const failed = outbox.filter((item) => item.action_type !== 'send_message' && item.status === 'failed');
    for (const item of failed) {
      item.status = 'pending';
      item.retry_count = 0;
      item.next_retry_at = null;
      item.last_error = null;
    }
    return failed.length;
  },
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
    removeDraftMessage(payload.id as string);
  },

  /* --- topics --------------------------------------------------------- */
  list_topics: () => topics.map((topic) => {
    const timeline = topicTimeline[topic.id] ?? [];
    const hidden = timeline.filter(item => {
      const thread = item.thread_id ? findThread(item.thread_id) : undefined;
      return thread && !accountVisible(thread.accountId);
    });
    return hidden.length ? { ...topic,
      item_count: Math.max(0, topic.item_count - hidden.length),
      unread_count: Math.max(0, topic.unread_count - hidden.reduce((sum, item) => sum + item.unread_count, 0)),
      last_sender_name: null, last_sender_email: null,
    } : { ...topic };
  }),
  get_topic_timeline: (payload) => (topicTimeline[payload.topicId as string] ?? []).filter(item => {
    if (!item.thread_id) return true;
    const thread = findThread(item.thread_id);
    return thread ? accountVisible(thread.accountId) : true;
  }),
  list_topic_memberships: (payload) => {
    const scope = input(payload);
    return topicLinks.filter((link) => link.item_type === scope.item_type
      && (scope.item_ids as string[]).includes(link.item_id)).map(({ topic_id, item_id }) => ({ topic_id, item_id }));
  },
  add_topic_items: (payload) => {
    const scope = input(payload);
    return (scope.item_ids as string[]).flatMap((id) => {
      const receipt = changeTopicLink(scope.topic_id as string, scope.item_type as string, id, true, String(scope.added_by ?? 'user'));
      return receipt ? [receipt] : [];
    });
  },
  remove_topic_item: (payload) => {
    const scope = input(payload);
    return changeTopicLink(scope.topic_id as string, scope.item_type as string, scope.item_id as string, false);
  },
  undo_topic_memberships: (payload) => {
    const scope = input(payload);
    const topicId = scope.topic_id as string;
    const itemType = scope.item_type as string;
    if (!topics.some((topic) => topic.id === topicId)) return [];
    return (scope.receipts as { item_id: string; revision: string }[]).flatMap((receipt) => {
      if (topicVersions.get(topicLinkKey(topicId, itemType, receipt.item_id)) !== receipt.revision) return [];
      const changed = changeTopicLink(topicId, itemType, receipt.item_id, scope.restore as boolean, 'user', scope.restore as boolean);
      return changed ? [receipt.item_id] : [];
    });
  },
  delete_topic: ({ id }) => {
    const index = topics.findIndex((topic) => topic.id === id);
    if (index >= 0) topics.splice(index, 1);
    delete topicTimeline[id as string];
    for (let i = topicLinks.length - 1; i >= 0; i--) {
      const link = topicLinks[i]!;
      if (link.topic_id !== id) continue;
      topicLinks.splice(i, 1);
    }
    for (const key of topicVersions.keys()) {
      if (JSON.parse(key)[0] !== id) continue;
      topicVersions.delete(key);
      removedTopicItems.delete(key);
    }
  },
  list_topic_files: () => [],

  /* --- composer ------------------------------------------------------- */
  send_message: (payload) => {
    const scope = input(payload);
    const accountId = payload.accountId as string;
    const recipients = (scope.to as string[] | undefined) ?? [];
    // Like the backend: Cc or Bcc alone is enough.
    const copies = [...((scope.cc as string[] | undefined) ?? []), ...((scope.bcc as string[] | undefined) ?? [])];
    if (recipients.length === 0 && copies.length === 0) throw new Error('missing recipient');

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
    const signed = mockSignedBody(scope);
    const sent: Message = {
      id: messageId,
      threadId,
      from: {
        name: (scope.from_name as string) || account?.displayName || '',
        email: (scope.from_address as string) || account?.email || '',
      },
      to: recipients.map((email) => ({ name: email, email })),
      subject: (scope.subject as string) ?? '',
      bodyText: signed.body,
      bodyHtml: signed.bodyHtml ?? undefined,
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

    // A sent draft is gone, from the drafts list and from its thread.
    const draftId = scope.draft_id as string | null;
    if (draftId) {
      const index = drafts.findIndex((draft) => draft.id === draftId);
      if (index >= 0) drafts.splice(index, 1);
      removeDraftMessage(draftId);
      removedDraftMessages.delete(draftId);
    }

    return { status: 'sent', thread_id: threadId, message_id: messageId, queued_id: null };
  },
  resolve_signature: (payload) => resolveMockSignature(input(payload)),
  list_signatures: (payload) => signatures.filter((signature) => !payload.accountId || signature.account_id === payload.accountId),
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
  run_imap_sync: () => ({
    accounts_synced: accounts.filter((account) => account.kind === 'email').length,
    accounts_failed: 0,
    folders_synced: 0,
    messages_synced: 0,
    backfill_pending_accounts: 0,
    backfill_remaining_messages: 0,
    backfill_total_messages: 0,
    failures: [],
  }),
  current_connection_state: () => ({
    unread_count: threads.filter(thread => accountVisible(thread.accountId) && thread.workflowState === 'inbox' && thread.unreadCount > 0 && !thread.isMuted && thread.mailboxNames?.some(name => name.toLowerCase() === 'inbox')).length,
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
  // No native notification gate retains responses here.
  mobile_notification_response_handled: () => undefined,
  list_plugins: () => [],
  list_plugin_metrics: () => [],
  list_plugin_audit_log: () => [],
  list_mailboxes: () => accounts.filter(account => account.kind === 'email').flatMap(account => {
    const rows = threads.filter(thread => thread.accountId === account.id);
    const folders = [...new Set(rows.flatMap(thread => thread.mailboxNames ?? []))];
    return folders.map(name => {
      const type = /^inbox$/i.test(name) ? 'inbox' : /^sent$/i.test(name) ? 'sent'
        : /^drafts?$/i.test(name) ? 'drafts' : /^trash$/i.test(name) ? 'trash'
          : /^archive$/i.test(name) ? 'archive' : 'custom';
      const included = rows.filter(thread => thread.mailboxNames?.includes(name)
        && !(type === 'inbox' && ['aside', 'done', 'snoozed'].includes(thread.workflowState)));
      return { id: `${account.id}:${name}`, account_id: account.id, account_email: account.email,
        name, mailbox_type: type, thread_count: included.length,
        unread_count: included.filter(thread => thread.unreadCount > 0).length };
    });
  }),
  list_smart_folders: () => [],
  list_follow_ups: () => [],
  list_invites: () => [],
  list_calendar_events: () => calendarEvents,
  list_calendar_sources: () => calendarSources,
  list_contacts: () => sortedContacts().slice(0, 500),
  search_contacts: ({ query, limit }) => {
    const term = String(query ?? '').trim().toLowerCase();
    if (!term) return [];
    return sortedContacts().filter((contact) => `${contact.email} ${contact.display_name ?? ''}`.toLowerCase().includes(term))
      .slice(0, Math.min(50, Math.max(1, Number(limit ?? 12))));
  },
  upsert_contact: ({ input: raw }) => {
    const input = raw as { id: string | null; email: string; display_name: string | null; notes: string | null; avatar_url: string | null };
    const email = input.email.trim().toLowerCase();
    if (!email.includes('@')) throw new Error('contact email is invalid');
    const previous = contacts.find((contact) => contact.email === email);
    const now = new Date().toISOString();
    if (previous) {
      previous.display_name = input.display_name?.trim() || previous.display_name;
      previous.notes = input.notes?.trim() || previous.notes;
      previous.frequency += 1;
      previous.last_contacted = now;
      previous.updated_at = now;
      return { ...previous };
    }
    const contact: BackendContact = {
      id: input.id ?? `contact-${email}`, email, display_name: input.display_name?.trim() || null,
      notes: input.notes?.trim() || null, avatar_url: input.avatar_url, avatar_source: 'manual',
      avatar_locked: input.avatar_url ? 1 : 0, avatar_updated_at: input.avatar_url ? now : null, frequency: 1, last_contacted: now,
      created_at: now, updated_at: now,
    };
    contacts.push(contact);
    return { ...contact };
  },
  delete_contact: ({ id }) => {
    const index = contacts.findIndex((contact) => contact.id === id);
    if (index >= 0) contacts.splice(index, 1);
  },
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
