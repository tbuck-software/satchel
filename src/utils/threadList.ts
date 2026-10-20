import type { Account, Participant, Thread, ThreadSmartBucket } from '@/types/mail';
import type { Priority, WorkflowState } from '@/types/workflow';
import type { BackendSmartInboxBucket, BackendThread } from '@/services/backend';
import type { InboxSenderBucket } from '@/stores/settings';
import { t } from '@/i18n/runtime';
import { localeFormatter } from '@/utils/listTime';
import { participantDisplayName } from '@/utils/mailIdentity';
import { isPersonalMailboxDomain, senderDomain } from '@/utils/senderDomains';
import { detectSenderSpoofing } from '@/utils/senderSpoofing';
import { detectCiProject } from '@/utils/ciNotifications';
import { detectNotificationBundle } from '@/utils/notificationBundles';

export type SmartInboxBucketKey = ThreadSmartBucket | 'other';
export type MailboxFilterKind = 'all' | 'section' | 'folder';

export interface MailboxFilter {
  kind: MailboxFilterKind;
  value: string | null;
}

export interface SimilarThreadGroup {
  id: string;
  ciProject?: string;
  senderName: string;
  senderEmail: string;
  title: string;
  previewText: string;
  count: number;
  unreadCount: number;
  lastMessageAt: string;
  threads: Thread[];
}

export interface SmartInboxBucketSummary {
  id: string;
  key: SmartInboxBucketKey;
  label: string;
  tone: 'priority' | 'pin' | 'calendar' | 'personal' | 'notification' | 'newsletter' | 'mail';
  count: number;
  unreadCount: number;
  threadIds: string[];
  previewText: string;
  previewItems: SmartInboxBucketPreviewItem[];
}

export interface SmartInboxBucketGroup extends SmartInboxBucketSummary {
  threads: Thread[];
}

export interface SmartInboxBucketPreviewItem {
  id: string;
  senderName: string;
  senderEmail: string;
  subject: string;
  unreadCount: number;
}

export interface NewSenderGroup {
  id: string;
  senderName: string;
  senderEmail: string;
  accountId: string;
  subject: string;
  previewText: string;
  count: number;
  unreadCount: number;
  lastMessageAt: string;
  /** `lastMessageAt` as epoch ms. */
  lastMessageAtMs: number;
  threadIds: string[];
  topThreadId: string;
  /** Display-name brand mismatch warning; null when the sender looks honest. */
  spoofingText: string | null;
}

export interface NewSenderDomainSuggestion {
  domain: string;
  senderCount: number;
  threadIds: string[];
}

export interface NewSenderStack {
  id: string;
  label: string;
  count: number;
  unreadCount: number;
  threadIds: string[];
  senders: NewSenderGroup[];
  /** Offered when several new senders share one organisation domain. */
  domainSuggestion: NewSenderDomainSuggestion | null;
}

export interface ContactThreadSummary {
  email: string;
  name: string;
  threadCount: number;
  messageCount: number;
  unreadCount: number;
  lastMessageAt: string;
  /** `lastMessageAt` as epoch ms, so the accumulator never reparses it. */
  lastMessageAtMs: number;
  previewText: string;
  threadIds: string[];
}

export type ThreadListEntry =
  | {
      kind: 'folder-row';
      id: string;
      folders: SmartInboxBucketSummary[];
    }
  | {
      kind: 'priority-card';
      id: string;
      bucket: SmartInboxBucketSummary;
      /** Newest first, capped at PRIORITY_CARD_MAX_THREADS full rows. */
      threads: Thread[];
      /** Remainder behind the "+N weitere" footer; based on the bucket count,
       *  which may exceed the locally loaded threads. */
      hiddenCount: number;
    }
  | {
      kind: 'date-header';
      id: string;
      label: string;
      count: number;
      threadIds: string[];
    }
  | {
      kind: 'bucket';
      id: string;
      bucket: SmartInboxBucketSummary;
    }
  | {
      kind: 'new-sender-stack';
      id: string;
      stack: NewSenderStack;
    }
  | {
      kind: 'group';
      id: string;
      group: SimilarThreadGroup;
    }
  | {
      kind: 'thread';
      id: string;
      thread: Thread;
    };

export interface SelectionModifiers {
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}


export function normalizePriority(priority: number): Priority {
  if (priority >= 3) return 3;
  if (priority >= 2) return 2;
  if (priority >= 1) return 1;
  return 0;
}

export function parseParticipants(raw: string): Participant[] {
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }

    const participants = parsed
      .map((value) => {
        if (typeof value === 'object' && value !== null) {
          const email = typeof value.email === 'string' ? value.email.trim() : '';
          const name = typeof value.name === 'string' ? value.name.trim() : '';
          if (!email) {
            return null;
          }
          return {
            name: name || email,
            email,
          } satisfies Participant;
        }

        if (typeof value !== 'string') {
          return null;
        }

        const [local] = value.split('@');
        if (!local) {
          return null;
        }
        const words = local.split(/[._-]+/).filter(Boolean);
        const display = words
          .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
          .join(' ');
        return {
          name: display || 'Unknown',
          email: value,
        } satisfies Participant;
      })
      .filter((item): item is Participant => Boolean(item));

    return participants;
  } catch {
    return [];
  }
}

export function cleanThreadSnippet(snippet: string, subject: string): string {
  const normalized = snippet.replace(/\r\n/g, '\n').replace(/\r/g, '\n').trim();
  if (!normalized) {
    return subject;
  }

  if (
    normalized.toLowerCase().includes('multipart message in mime format') ||
    /^content-type:/i.test(normalized) ||
    /^--[-_A-Za-z0-9.=]{6,}/.test(normalized)
  ) {
    return subject;
  }

  return normalized;
}

export function normalizedSubjectWhitespace(subject: string): string {
  return subject.replace(/\s+/g, ' ').trim();
}

export function primarySenderEmail(thread: Thread): string {
  return thread.participants[0]?.email.trim().toLowerCase() ?? '';
}

export function normalizedParticipantEmail(email?: string | null): string {
  return email?.trim().toLowerCase() ?? '';
}

export function threadHasParticipant(thread: Thread, email: string): boolean {
  return thread.participants.some((participant) => normalizedParticipantEmail(participant.email) === email);
}

/**
 * Outbound recipients of a thread, deduplicated and named.
 *
 * Contact suggestions must be based on known outbound recipients only.
 * Falling back to every thread participant promotes inbound bots and BCC
 * senders to "frequent contacts" merely because a thread also has sent mail.
 *
 * `only` restricts the result to a known set of addresses; callers that need
 * a handful of contacts skip the per-thread bookkeeping for everyone else.
 */
export function threadSentParticipants(
  thread: Thread,
  ownEmails: Set<string>,
  only?: ReadonlySet<string>,
): Participant[] {
  const source = thread.sentParticipants ?? [];
  if (source.length === 0) return [];

  const wanted = (email: string) => !only || only.has(email);
  if (only && !source.some((participant) => only.has(normalizedParticipantEmail(participant.email)))) {
    return [];
  }

  const namesByEmail = new Map<string, Participant>();
  const collectName = (participant: Participant) => {
    const email = normalizedParticipantEmail(participant.email);
    if (!email || email.endsWith('@satchel.local') || ownEmails.has(email) || !wanted(email)) {
      return;
    }
    const existing = namesByEmail.get(email);
    if (!existing || contactDisplayName(existing.name, participant.name, email) !== existing.name) {
      namesByEmail.set(email, {
        name: contactDisplayName(existing?.name ?? '', participant.name, email),
        email,
      });
    }
  };
  for (const participant of thread.participants) collectName(participant);
  for (const participant of source) collectName(participant);

  const seen = new Set<string>();
  const recipients: Participant[] = [];
  for (const participant of source) {
    const email = normalizedParticipantEmail(participant.email);
    if (!email || email.endsWith('@satchel.local') || ownEmails.has(email) || seen.has(email) || !wanted(email)) {
      continue;
    }
    seen.add(email);
    recipients.push(namesByEmail.get(email) ?? { name: email, email });
  }
  return recipients;
}

export function isAutomatedContact(participant: Participant): boolean {
  const value = `${participant.name} ${participant.email}`.toLowerCase();
  const localPart = normalizedParticipantEmail(participant.email).split('@')[0] ?? '';
  const roleAddress = /^(info|support|service|contact|hello|booking|bookings|reservation|reservations|orders?|newsletter|delivery|notifications?|mail|post)$/;
  return roleAddress.test(localPart)
    || /(^|[\s@._-])(no-?reply|do-?not-?reply|mailer-daemon|notifications?|automated|robot|bot)([\s@._-]|$)/.test(value);
}

export function isMachineSender(participant?: Participant): boolean {
  if (!participant) return false;
  const value = `${participant.name} ${participant.email}`.toLowerCase();
  return /^gitlab@/i.test(participant.email.trim())
    || /(^|[\s@._-])(no-?reply|do-?not-?reply|mailer-daemon|notifications?|automated|robot|bot|newsletter|delivery)([\s@._-]|$)/.test(value);
}

export function contactDisplayName(current: string, next: string, email: string): string {
  const normalizedCurrent = current.trim();
  const normalizedNext = next.trim();
  if (!normalizedCurrent || normalizedCurrent === email) {
    return normalizedNext || email;
  }
  if (normalizedNext && normalizedNext !== email && normalizedNext.length > normalizedCurrent.length) {
    return normalizedNext;
  }
  return normalizedCurrent;
}

function accumulateContact(
  contacts: Map<string, ContactThreadSummary>,
  thread: Thread,
  participant: Participant,
): void {
  const email = participant.email;
  const existing = contacts.get(email);
  if (!existing) {
    contacts.set(email, {
      email,
      name: participant.name || email,
      threadCount: 1,
      messageCount: thread.messageCount,
      unreadCount: thread.unreadCount,
      lastMessageAt: thread.lastMessageAt,
      lastMessageAtMs: thread.lastMessageAtMs,
      previewText: normalizedSubjectWhitespace(thread.normalizedSubject ?? thread.subject) || cleanThreadSnippet(thread.snippet, thread.subject),
      threadIds: [thread.id],
    });
    return;
  }

  existing.name = contactDisplayName(existing.name, participant.name, email);
  existing.threadCount += 1;
  existing.messageCount += thread.messageCount;
  existing.unreadCount += thread.unreadCount;
  existing.threadIds.push(thread.id);
  if (thread.lastMessageAtMs > existing.lastMessageAtMs) {
    existing.lastMessageAt = thread.lastMessageAt;
    existing.lastMessageAtMs = thread.lastMessageAtMs;
    existing.previewText = normalizedSubjectWhitespace(thread.normalizedSubject ?? thread.subject) || cleanThreadSnippet(thread.snippet, thread.subject);
  }
}

/** Group headers and broken parses yield name-only "addresses" like "buck". */
function isContactAddress(email: string): boolean {
  return email.includes('@');
}

function collectContacts(threads: readonly Thread[], ownEmails: Set<string>): Map<string, ContactThreadSummary> {
  const contacts = new Map<string, ContactThreadSummary>();

  // One pass per thread. The former pre-pass collected the same addresses a
  // second time; `threadSentParticipants` already yields unique recipients
  // without own or local addresses, so the extra set was always a no-op filter.
  for (const thread of threads) {
    for (const participant of threadSentParticipants(thread, ownEmails)) {
      if (!isContactAddress(participant.email) || isAutomatedContact(participant)) {
        continue;
      }
      accumulateContact(contacts, thread, participant);
    }
  }

  return contacts;
}

/** Most threads first, then most messages, then most recent, then by name. */
function compareContacts(a: ContactThreadSummary, b: ContactThreadSummary): number {
  if (b.threadCount !== a.threadCount) {
    return b.threadCount - a.threadCount;
  }
  if (b.messageCount !== a.messageCount) {
    return b.messageCount - a.messageCount;
  }
  const dateDelta = b.lastMessageAtMs - a.lastMessageAtMs;
  if (dateDelta !== 0) {
    return dateDelta;
  }
  return a.name.localeCompare(b.name);
}

export function buildContactSummaries(threads: Thread[], ownEmails: Set<string>): ContactThreadSummary[] {
  return [...collectContacts(threads, ownEmails).values()].sort(compareContacts);
}

/**
 * The first `limit` entries `buildContactSummaries` would return, skipping
 * `excludedEmails`.
 *
 * The sidebar shows five contacts. Sorting every known address for that
 * on each thread mutation was the larger half of its cost; a bounded
 * insertion keeps the selection linear in the number of contacts.
 */
export function buildTopContactSummaries(
  threads: readonly Thread[],
  ownEmails: Set<string>,
  limit: number,
  excludedEmails: ReadonlySet<string> = new Set(),
): ContactThreadSummary[] {
  const top: ContactThreadSummary[] = [];
  if (limit <= 0) return top;
  for (const contact of collectContacts(threads, ownEmails).values()) {
    if (excludedEmails.has(contact.email.toLowerCase())) continue;
    const last = top[top.length - 1];
    if (top.length >= limit && last && compareContacts(contact, last) >= 0) continue;
    let position = top.length;
    while (position > 0 && compareContacts(contact, top[position - 1]!) < 0) position -= 1;
    top.splice(position, 0, contact);
    if (top.length > limit) top.pop();
  }
  return top;
}

/**
 * Summaries for a handful of known addresses — the sidebar needs pinned
 * contacts resolved across accounts, which used to cost a second full
 * projection over every thread.
 */
export function buildPinnedContactSummaries(
  threads: Thread[],
  ownEmails: Set<string>,
  pinnedEmails: readonly string[],
): Map<string, ContactThreadSummary> {
  const contacts = new Map<string, ContactThreadSummary>();
  const wanted = new Set(
    pinnedEmails.map(normalizedParticipantEmail).filter(isContactAddress),
  );
  if (wanted.size === 0) {
    return contacts;
  }

  for (const thread of threads) {
    for (const participant of threadSentParticipants(thread, ownEmails, wanted)) {
      if (isAutomatedContact(participant)) {
        continue;
      }
      accumulateContact(contacts, thread, participant);
    }
  }

  return contacts;
}

export interface NewSenderReviewInputs {
  senderBuckets: Record<string, InboxSenderBucket>;
  acceptedSenders: Record<string, true>;
  acceptedDomains: Record<string, true>;
  senderReviewStartedAt: string | null;
  dismissedNewSenders: Record<string, string>;
  ownEmails: Set<string>;
}

/**
 * Build the "is this an unreviewed sender?" test once per input set.
 *
 * The review start and the dismissal dates are parsed here, not per thread:
 * the list projections run this over every visible thread on each mutation.
 */
export function newSenderPredicate({
  senderBuckets,
  acceptedSenders,
  acceptedDomains,
  senderReviewStartedAt,
  dismissedNewSenders,
  ownEmails,
}: NewSenderReviewInputs): (thread: Thread) => boolean {
  const reviewStart = senderReviewStartedAt ? Date.parse(senderReviewStartedAt) : NaN;
  const dismissedThrough = new Map<string, number>();
  function dismissedThroughFor(email: string): number {
    let value = dismissedThrough.get(email);
    if (value === undefined) {
      value = Date.parse(dismissedNewSenders[email] ?? '');
      dismissedThrough.set(email, value);
    }
    return value;
  }

  return (thread) => {
    if (thread.isStarred || thread.priority >= 2 || thread.hasInvite) {
      return false;
    }

    if (thread.senderKnown) {
      return false;
    }

    const senderEmail = primarySenderEmail(thread);
    if (!senderEmail || senderEmail.endsWith('@satchel.local')) {
      return false;
    }

    // One of our own addresses is never a sender to review. It reaches the top of
    // the participant list whenever the thread leads with our own message.
    if (ownEmails.has(senderEmail)) {
      return false;
    }

    if (thread.sentParticipants?.some((participant) => normalizedParticipantEmail(participant.email) === senderEmail)) {
      return false;
    }

    // The category never vetoes the review: bulk signals like List-Unsubscribe
    // also cloak phishing, so an unknown sender must stay reviewable even when
    // classified as promotions/updates/social. Machine-shaped senders and the
    // allowlists below remain the noise filters.
    if (isMachineSender(thread.participants[0])) {
      return false;
    }

    const lastMessage = thread.lastMessageAtMs;
    if (Number.isFinite(reviewStart) && Number.isFinite(lastMessage) && lastMessage <= reviewStart) {
      return false;
    }

    const dismissed = dismissedThroughFor(senderEmail);
    if (Number.isFinite(dismissed) && Number.isFinite(lastMessage) && lastMessage <= dismissed) {
      return false;
    }

    return (
      !senderBuckets[senderEmail] &&
      !acceptedSenders[senderEmail] &&
      !acceptedDomains[senderDomain(senderEmail)]
    );
  };
}

export function isNewSenderThread(
  thread: Thread,
  senderBuckets: Record<string, InboxSenderBucket>,
  acceptedSenders: Record<string, true>,
  acceptedDomains: Record<string, true>,
  senderReviewStartedAt: string | null,
  dismissedNewSenders: Record<string, string>,
  ownEmails: Set<string>,
): boolean {
  return newSenderPredicate({
    senderBuckets,
    acceptedSenders,
    acceptedDomains,
    senderReviewStartedAt,
    dismissedNewSenders,
    ownEmails,
  })(thread);
}

/**
 * Ids of the threads that belong in the new-sender review.
 *
 * Both the review stack and the smart buckets need this split; computing it
 * once lets each of them read a set instead of rerunning the test.
 */
export function collectNewSenderThreadIds(
  threads: readonly Thread[],
  inputs: NewSenderReviewInputs,
): Set<string> {
  const isNewSender = newSenderPredicate(inputs);
  const ids = new Set<string>();
  for (const thread of threads) {
    if (isNewSender(thread)) ids.add(thread.id);
  }
  return ids;
}

export function bucketLabel(key: SmartInboxBucketKey): string {
  switch (key) {
    case 'priority':
      return t('thread.bucket.priority');
    case 'pinned':
      return t('thread.bucket.pinned');
    case 'invitations':
      return t('thread.bucket.invitations');
    case 'personal':
      return t('thread.bucket.personal');
    case 'notifications':
      return t('thread.bucket.notifications');
    case 'newsletter':
      return t('thread.bucket.newsletter');
    default:
      return t('thread.bucket.other');
  }
}

export function bucketTone(key: SmartInboxBucketKey): SmartInboxBucketSummary['tone'] {
  switch (key) {
    case 'priority':
      return 'priority';
    case 'pinned':
      return 'pin';
    case 'invitations':
      return 'calendar';
    case 'personal':
      return 'personal';
    case 'notifications':
      return 'notification';
    case 'newsletter':
      return 'newsletter';
    default:
      return 'mail';
  }
}

export function classifySmartBucket(thread: Thread): SmartInboxBucketKey {
  if (thread.priority >= 2) {
    return 'priority';
  }

  if (thread.isStarred) {
    return 'pinned';
  }

  if (thread.hasInvite) {
    return 'invitations';
  }

  // Local flags above support optimistic actions. The remaining assignment
  // comes from the same backend rule as counts and focused-folder queries.
  if (thread.smartBucket === 'notifications' || thread.smartBucket === 'newsletter' || thread.smartBucket === 'personal') {
    return thread.smartBucket;
  }

  if (isMachineSender(thread.participants[0]) && (!thread.category || thread.category === 'personal')) {
    return 'notifications';
  }

  switch (thread.category) {
    case 'updates':
      return 'notifications';
    case 'promotions':
      return 'newsletter';
    case 'social':
      return 'notifications';
    case 'personal':
    case null:
    case undefined:
      return 'personal';
    default:
      return 'other';
  }
}

export function buildBucketPreview(threads: Thread[]): string {
  return threads
    .slice(0, 4)
    .map((thread) => thread.participants[0]?.name || thread.participants[0]?.email || thread.subject)
    .filter(Boolean)
    .join(' · ');
}

export function buildBucketPreviewItems(threads: Thread[]): SmartInboxBucketPreviewItem[] {
  return threads.slice(0, 4).map((thread) => ({
    id: thread.id,
    senderName: thread.participants[0]?.name || thread.participants[0]?.email || thread.subject,
    senderEmail: thread.participants[0]?.email ?? '',
    subject: normalizedSubjectWhitespace(thread.normalizedSubject ?? thread.subject) || thread.subject,
    unreadCount: thread.unreadCount,
  }));
}

/** Ignore labelled build/version counters and trailing revisions; keep project, branch and issue IDs. */
function bundleSubject(subject: string): string {
  return normalizedSubjectWhitespace(subject).normalize('NFC').toLowerCase()
    .replace(/^(.+\s+\|\s+.+?)\s+\|\s+[a-f0-9]{7,40}$/, '$1 | <revision>')
    .replace(/\bversion\s+\d+\.\d+\.\d+(?:[-+][\w.-]+)?(?:\s*\(\d+\))?/g, 'version <version>')
    .replace(/\b(build|pipeline)\s+#?\s*\d+\b/g, '$1 <number>');
}

/** Membership depends on each mail's identity, never on neighbouring rows. */
export function buildSimilarGroups(inputThreads: Thread[], dateSection = ''): SimilarThreadGroup[] {
  const byIdentity = new Map<string, { threads: Thread[]; ciProject?: string }>();
  for (const thread of inputThreads) {
    const senderEmail = primarySenderEmail(thread);
    const subject = bundleSubject(thread.normalizedSubject ?? thread.subject);
    if (!senderEmail || !subject) continue;
    const project = detectCiProject(thread, normalizedSubjectWhitespace(thread.normalizedSubject ?? thread.subject));
    const notification = detectNotificationBundle(senderEmail, thread.normalizedSubject ?? thread.subject);
    if (project && !Number.isFinite(thread.lastMessageAtMs)) continue;
    const senderName = thread.participants[0]?.name.normalize('NFC').trim().replace(/\s+/g, ' ').toLowerCase() ?? '';
    // Structured encoding preserves punctuation and separates account/date scopes.
    let identity: string;
    if (project) {
      const date = new Date(thread.lastMessageAtMs);
      const day = [date.getFullYear(), date.getMonth(), date.getDate()];
      const path = project.path.normalize('NFC');
      identity = JSON.stringify([
        dateSection, thread.accountId, 'ci', project.forge, project.host,
        project.forge === 'github' ? path.toLowerCase() : path, day,
      ]);
    } else if (notification) {
      identity = JSON.stringify([dateSection, thread.accountId, 'notification', senderEmail, senderName, ...notification]);
    } else {
      identity = JSON.stringify([dateSection, thread.accountId, senderEmail, senderName, subject]);
    }
    const members = byIdentity.get(identity);
    if (members) members.threads.push(thread);
    else byIdentity.set(identity, { threads: [thread], ciProject: project?.path });
  }

  const groups: SimilarThreadGroup[] = [];
  for (const [identity, { threads: members, ciProject }] of byIdentity) {
    if (members.length < 2) continue;
    const latest = members[0]!;
    const sender = latest.participants[0]!;
    const title = normalizedSubjectWhitespace(latest.normalizedSubject ?? latest.subject) || latest.subject;
    groups.push({
      id: `grp-${encodeURIComponent(identity)}`,
      ciProject,
      senderName: sender.name || sender.email,
      senderEmail: sender.email,
      title,
      previewText: cleanThreadSnippet(latest.snippet, latest.subject),
      count: members.length,
      unreadCount: members.reduce((sum, thread) => sum + thread.unreadCount, 0),
      lastMessageAt: latest.lastMessageAt,
      threads: members,
    });
  }
  return groups;
}

/**
 * Suggest allowing a whole organisation domain when several senders share it —
 * `info@` and `rechnung@` of the same shop are one decision, not two. Freemail
 * domains never qualify: there a shared domain means nothing.
 */
export function buildDomainSuggestion(senders: NewSenderGroup[]): NewSenderDomainSuggestion | null {
  const byDomain = new Map<string, NewSenderGroup[]>();
  for (const sender of senders) {
    const domain = senderDomain(sender.senderEmail);
    if (!domain || isPersonalMailboxDomain(domain)) {
      continue;
    }
    const domainSenders = byDomain.get(domain) ?? [];
    domainSenders.push(sender);
    byDomain.set(domain, domainSenders);
  }

  let best: { domain: string; senders: NewSenderGroup[] } | null = null;
  for (const [domain, domainSenders] of byDomain) {
    if (domainSenders.length < 2) {
      continue;
    }
    if (!best || domainSenders.length > best.senders.length) {
      best = { domain, senders: domainSenders };
    }
  }

  if (!best) {
    return null;
  }

  return {
    domain: best.domain,
    senderCount: best.senders.length,
    threadIds: best.senders.flatMap((sender) => sender.threadIds),
  };
}

export function buildNewSenderStack(
  inputThreads: Thread[],
  newSenderThreadIds: ReadonlySet<string>,
): NewSenderStack | null {
  const buckets = new Map<string, Thread[]>();
  if (newSenderThreadIds.size === 0) return null;

  for (const thread of inputThreads) {
    if (!newSenderThreadIds.has(thread.id)) {
      continue;
    }

    const email = primarySenderEmail(thread);
    const senderThreads = buckets.get(email) ?? [];
    senderThreads.push(thread);
    buckets.set(email, senderThreads);
  }

  const senders = [...buckets.entries()]
    .map(([senderEmail, senderThreads]) => {
      const sortedThreads = [...senderThreads].sort((a, b) => b.lastMessageAtMs - a.lastMessageAtMs);
      const topThread = sortedThreads[0];
      const sender = topThread?.participants[0];
      if (!topThread || !sender) {
        return null;
      }
      return {
        id: `new-sender-${senderEmail.replace(/[^a-z0-9]+/g, '-')}`,
        senderName: sender?.name || senderEmail,
        senderEmail,
        accountId: topThread.accountId,
        subject: normalizedSubjectWhitespace(topThread.normalizedSubject ?? topThread.subject) || topThread.subject,
        previewText: cleanThreadSnippet(topThread.snippet, topThread.subject),
        count: sortedThreads.length,
        unreadCount: sortedThreads.reduce((sum, thread) => sum + thread.unreadCount, 0),
        lastMessageAt: topThread.lastMessageAt,
        lastMessageAtMs: topThread.lastMessageAtMs,
        threadIds: sortedThreads.map((thread) => thread.id),
        topThreadId: topThread.id,
        spoofingText: detectSenderSpoofing(sender?.name, senderEmail)?.text ?? null,
      } satisfies NewSenderGroup;
    })
    .filter((group): group is NewSenderGroup => group !== null)
    .sort((a, b) => b.lastMessageAtMs - a.lastMessageAtMs);

  if (senders.length === 0) {
    return null;
  }

  return {
    id: 'new-sender-stack',
    label: t('thread.newSenders.title'),
    count: senders.length,
    unreadCount: senders.reduce((sum, sender) => sum + sender.unreadCount, 0),
    threadIds: senders.flatMap((sender) => sender.threadIds),
    senders,
    domainSuggestion: buildDomainSuggestion(senders),
  };
}

/**
 * Merge two lists that are each already in list order.
 *
 * The head refresh re-sorted everything loaded on every sync event, although
 * only the fresh page had moved. Ties keep the left list first, which is what
 * sorting the concatenation did.
 */
export function mergeOrdered(
  left: readonly Thread[],
  right: readonly Thread[],
  compare: (a: Thread, b: Thread) => number,
): Thread[] {
  const merged: Thread[] = [];
  let leftIndex = 0;
  let rightIndex = 0;
  while (leftIndex < left.length && rightIndex < right.length) {
    merged.push(
      compare(left[leftIndex]!, right[rightIndex]!) <= 0
        ? left[leftIndex++]!
        : right[rightIndex++]!,
    );
  }
  while (leftIndex < left.length) merged.push(left[leftIndex++]!);
  while (rightIndex < right.length) merged.push(right[rightIndex++]!);
  return merged;
}

export function buildThreadEntries(inputThreads: Thread[], similarGroups: SimilarThreadGroup[]): ThreadListEntry[] {
  const groupByLeadThreadId = new Map<string, SimilarThreadGroup>();
  const groupedThreadIds = new Set<string>();

  // The lead is the group's first member by construction (see buildSimilarGroups),
  // so no search over the section is needed to find it.
  similarGroups.forEach((group) => {
    const leadThread = group.threads[0];
    if (!leadThread) {
      return;
    }
    groupByLeadThreadId.set(leadThread.id, group);
    group.threads.forEach((thread) => groupedThreadIds.add(thread.id));
  });

  return inputThreads.flatMap<ThreadListEntry>((thread): ThreadListEntry[] => {
    const leadGroup = groupByLeadThreadId.get(thread.id);
    if (leadGroup) {
      return [
        {
          kind: 'group',
          id: leadGroup.id,
          group: leadGroup,
        },
      ];
    }

    if (groupedThreadIds.has(thread.id)) {
      return [];
    }

    return [
      {
        kind: 'thread',
        id: `thread-${thread.id}`,
        thread,
      },
    ];
  });
}

export function buildFolderDateGroups(inputThreads: Thread[]): Array<{
  key: string;
  label: string;
  threads: Thread[];
}> {
  const now = new Date();
  const startTodayDate = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startYesterdayDate = new Date(startTodayDate);
  startYesterdayDate.setDate(startYesterdayDate.getDate() - 1);
  const startLastWeekDate = new Date(startTodayDate);
  startLastWeekDate.setDate(startLastWeekDate.getDate() - 7);

  const startToday = startTodayDate.getTime();
  const startYesterday = startYesterdayDate.getTime();
  const startLastWeek = startLastWeekDate.getTime();
  const monthFormatter = localeFormatter('month', { month: 'long' });
  const monthYearFormatter = localeFormatter('monthYear', { month: 'long', year: 'numeric' });
  const monthGroups = new Map<string, { key: string; label: string; threads: Thread[] }>();
  const todayThreads: Thread[] = [];
  const yesterdayThreads: Thread[] = [];
  const lastWeekThreads: Thread[] = [];

  // Classify on the precomputed timestamp; only the month branch needs a Date
  // object. This projection is on the hot path for every mailbox update.
  for (const thread of inputThreads) {
    const time = thread.lastMessageAtMs;
    if (!Number.isFinite(time)) continue;
    if (time >= startToday) {
      todayThreads.push(thread);
      continue;
    }
    if (time >= startYesterday) {
      yesterdayThreads.push(thread);
      continue;
    }
    if (time >= startLastWeek) {
      lastWeekThreads.push(thread);
      continue;
    }

    const date = new Date(time);
    const year = date.getFullYear();
    const month = date.getMonth();
    const key = `month-${year}-${String(month + 1).padStart(2, '0')}`;
    const existing = monthGroups.get(key);
    if (existing) {
      existing.threads.push(thread);
      continue;
    }

    monthGroups.set(key, {
      key,
      label: year === now.getFullYear()
        ? monthFormatter.format(date)
        : monthYearFormatter.format(date),
      threads: [thread],
    });
  }

  const recentGroups = [
    { key: 'today' as const, label: t('common.today'), threads: todayThreads },
    { key: 'yesterday' as const, label: t('common.yesterday'), threads: yesterdayThreads },
    { key: 'last-week' as const, label: t('time.lastWeek'), threads: lastWeekThreads },
  ];

  return [
    ...recentGroups.filter((group) => group.threads.length > 0),
    ...[...monthGroups.values()].sort((left, right) => right.key.localeCompare(left.key)),
  ];
}

/**
 * A date section together with its sender bundles.
 *
 * Both list projections need exactly this, and callers that need both must
 * build it once: the sections and their bundles used to be recomputed twice
 * per list recompute, over the same input.
 */
export interface ThreadDateSection {
  key: string;
  label: string;
  threads: Thread[];
  similarGroups: SimilarThreadGroup[];
}

/**
 * Bundles must be built per date section: a group that crosses a section
 * boundary would otherwise produce one bundle whose card can only render in a
 * single section, silently swallowing the threads on the other side.
 */
export function buildThreadDateSections(inputThreads: Thread[]): ThreadDateSection[] {
  return buildFolderDateGroups(inputThreads).map((group) => ({
    ...group,
    similarGroups: buildSimilarGroups(group.threads, group.key),
  }));
}

export function sectionSimilarGroups(sections: readonly ThreadDateSection[]): SimilarThreadGroup[] {
  return sections.flatMap((section) => section.similarGroups);
}

function sectionRows(section: ThreadDateSection): ThreadListEntry[] {
  return [
    {
      kind: 'date-header',
      id: `date-${section.key}`,
      label: section.label,
      count: section.threads.length,
      threadIds: section.threads.map((thread) => thread.id),
    },
    ...buildThreadEntries(section.threads, section.similarGroups),
  ];
}

export function sectionEntries(sections: readonly ThreadDateSection[]): ThreadListEntry[] {
  return sections.flatMap(sectionRows);
}

export function buildDateScopedSimilarGroups(inputThreads: Thread[]): SimilarThreadGroup[] {
  return sectionSimilarGroups(buildThreadDateSections(inputThreads));
}

export function buildDateGroupedEntries(inputThreads: Thread[]): ThreadListEntry[] {
  return sectionEntries(buildThreadDateSections(inputThreads));
}

/** Full rows shown in the priority card before the rest folds behind "+N more". */
export const PRIORITY_CARD_MAX_THREADS = 3;

export function buildPriorityCardEntry(
  bucket: SmartInboxBucketGroup,
): Extract<ThreadListEntry, { kind: 'priority-card' }> {
  const threads = bucket.threads.slice(0, PRIORITY_CARD_MAX_THREADS);
  return {
    kind: 'priority-card',
    id: 'priority-card',
    bucket,
    threads,
    // `bucket.count` is the server's answer, `threads` come from the loaded
    // window, and the two disagree when the SQL classifier sees a thread the
    // local one does not. Counting the fold from the rows the card actually
    // holds keeps "+N more" a promise about rows that exist behind it: with no
    // rows there is no fold, and the caller keeps the bucket off the card.
    hiddenCount: threads.length === 0
      ? 0
      : Math.max(0, bucket.count - threads.length),
  };
}

/**
 * Smart inbox layout: the smart folder cards live inside the "Heute" section,
 * below today's plain mails — the day reads as "what arrived today, and where
 * the rest of it was filed". The "Heute" header therefore exists even on a day
 * without plain mail, and its header count and bulk action cover only the
 * visible rows, never the folders' contents.
 */
export function buildSmartDateGroupedEntries(
  inputThreads: Thread[],
  folderRow: Extract<ThreadListEntry, { kind: 'folder-row' }> | null,
): ThreadListEntry[] {
  if (!folderRow) {
    return buildDateGroupedEntries(inputThreads);
  }

  const sections = buildThreadDateSections(inputThreads);
  const todaySection = sections.find((section) => section.key === 'today')
    ?? { key: 'today', label: t('common.today'), threads: [], similarGroups: [] };
  const laterSections = sections.filter((section) => section.key !== 'today');

  return [
    ...sectionRows(todaySection),
    folderRow,
    ...laterSections.flatMap(sectionRows),
  ];
}

export function buildSmartInboxBuckets(
  inputThreads: Thread[],
  newSenderThreadIds: ReadonlySet<string>,
  serverBuckets?: BackendSmartInboxBucket[],
): SmartInboxBucketGroup[] {
  const bucketOrder: SmartInboxBucketKey[] = [
    'priority',
    'pinned',
    'invitations',
    'notifications',
    'newsletter',
  ];
  const buckets = new Map<SmartInboxBucketKey, Thread[]>();

  for (const thread of inputThreads) {
    if (newSenderThreadIds.has(thread.id)) {
      continue;
    }

    const bucketKey = classifySmartBucket(thread);
    if (!bucketOrder.includes(bucketKey)) {
      continue;
    }

    const bucketThreads = buckets.get(bucketKey) ?? [];
    bucketThreads.push(thread);
    buckets.set(bucketKey, bucketThreads);

    // Priority and pinning are independent signals. A pinned priority thread
    // remains visible in the priority group and is also discoverable under
    // "Angeheftet".
    if (thread.isStarred && bucketKey !== 'pinned') {
      const pinnedThreads = buckets.get('pinned') ?? [];
      pinnedThreads.push(thread);
      buckets.set('pinned', pinnedThreads);
    }
  }

  const groups: SmartInboxBucketGroup[] = [];
  for (const bucketKey of bucketOrder) {
    const bucketThreads = buckets.get(bucketKey) ?? [];
    const serverBucket = serverBuckets?.find((entry) => entry.key === bucketKey);
    // Once the summary has answered for this scope, it decides which rows
    // exist. Opening one runs `list_threads` with the same `smart_bucket`
    // filter, so a row the server does not know resolves to an empty page and
    // replaces the whole list with nothing. The local classifier reads sender
    // names and domains the SQL one does not, and every bucket it invents on
    // its own strands the user on a blank Inbox.
    if (serverBuckets ? !serverBucket : bucketThreads.length === 0) {
      continue;
    }

    const sortedThreads = [...bucketThreads].sort(
      (a, b) => b.lastMessageAtMs - a.lastMessageAtMs,
    );

    groups.push({
      id: `bucket-${bucketKey}`,
      key: bucketKey,
      label: bucketLabel(bucketKey),
      tone: bucketTone(bucketKey),
      count: serverBucket?.count ?? sortedThreads.length,
      unreadCount:
        serverBucket?.unread_count
        ?? sortedThreads.reduce((sum, thread) => sum + thread.unreadCount, 0),
      threadIds: sortedThreads.map((thread) => thread.id),
      previewText: serverBucket
        ? serverBucket.preview_items.map((item) => item.sender_name).join(' · ')
        : buildBucketPreview(sortedThreads),
      previewItems: serverBucket
        ? serverBucket.preview_items.map((item) => ({
            id: item.id,
            senderName: item.sender_name,
            senderEmail: item.sender_email,
            subject: item.subject,
            unreadCount: item.unread_count,
          }))
        : buildBucketPreviewItems(sortedThreads),
      threads: sortedThreads,
    });
  }

  return groups;
}

export function parseMailboxNames(raw: string): string[] {
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return parsed
      .map((value) => (typeof value === 'string' ? value.trim() : ''))
      .filter(Boolean);
  } catch {
    return [];
  }
}

export function classifyMailboxName(name: string): MailboxFilter['value'] {
  const lower = name.toLowerCase();
  if (lower.includes('archive') || lower.includes('archiv')) return 'archive';
  if (lower.includes('sent')) return 'sent';
  if (lower.includes('draft')) return 'drafts';
  if (lower.includes('trash') || lower.includes('deleted')) return 'trash';
  if (lower.includes('spam') || lower.includes('junk')) return 'spam';
  if (lower === 'inbox' || lower.startsWith('inbox.')) return 'inbox';
  if (lower.includes('notif') || lower.includes('updates') || lower.includes('forum')) {
    return 'notifications';
  }
  if (lower.includes('blocked')) return 'blocked';
  return null;
}

export function toThreadModel(input: BackendThread, accounts: readonly Account[]): Thread {
  const participants = parseParticipants(input.participants).map((participant) => ({
    ...participant,
    name: participantDisplayName(accounts, participant),
  }));
  const sentParticipants =
    input.sent_participants === undefined ? undefined : parseParticipants(input.sent_participants);
  const snippet = cleanThreadSnippet(input.snippet || input.subject, input.subject);
  const mailboxNames = parseMailboxNames(input.mailbox_names);
  const lastMessageAt = input.last_message_date || new Date().toISOString();

  return {
    id: input.id,
    accountId: input.account_id,
    subject: input.subject,
    normalizedSubject: input.normalized_subject,
    participants: participants.length > 0 ? participants : [{ name: 'Unknown', email: 'unknown@satchel.local' }],
    sentParticipants,
    snippet,
    lastMessageAt,
    sortDate: input.sort_date ?? lastMessageAt,
    lastMessageAtMs: Date.parse(lastMessageAt),
    messageCount: Math.max(1, input.message_count || 1),
    unreadCount: input.unread_count,
    hasAttachments: input.has_attachments,
    hasDraft: Boolean(input.has_draft),
    draftSnippet: input.draft_snippet?.trim() || undefined,
    workflowState: input.workflow_state,
    priority: normalizePriority(input.priority),
    boardPosition: input.board_position ?? undefined,
    mailboxNames,
    tags: input.tags ?? [],
    assignee: input.assignee ?? undefined,
    sharedWith: input.shared_with ?? [],
    category: (input.category ?? null) as Thread['category'],
    smartBucket: input.smart_bucket,
    isMuted: Boolean(input.is_muted),
    isStarred: Boolean(input.is_starred),
    snoozeUntil: input.snooze_until ?? undefined,
    senderKnown: Boolean(input.sender_known),
    hasInvite: Boolean(input.has_invite),
    inviteStatus: input.invite_status ?? undefined,
    inviteCancelled: Boolean(input.invite_cancelled),
  };
}


export function matchesMailboxFilter(thread: Thread, filter: MailboxFilter): boolean {
  if (filter.kind === 'all' || !filter.value) {
    return true;
  }

  if (filter.kind === 'folder') {
    return thread.mailboxNames.includes(filter.value);
  }

  if (filter.value === 'archive') {
    return thread.workflowState === 'done';
  }

  if (filter.value === 'snoozed') {
    return thread.workflowState === 'snoozed';
  }

  if (filter.value === 'aside') {
    return thread.workflowState === 'aside';
  }

  if (
    thread.workflowState === 'done'
    || thread.workflowState === 'snoozed'
    || thread.workflowState === 'aside'
  ) {
    return false;
  }

  return thread.mailboxNames.some((name) => classifyMailboxName(name) === filter.value);
}

export function isVisibleInCurrentScope(
  thread: Thread,
  workflow: WorkflowState | 'all',
  mailbox: MailboxFilter,
): boolean {
  if (workflow === 'done' || workflow === 'snoozed' || workflow === 'aside') {
    return true;
  }

  if (
    mailbox.kind === 'section' &&
    (mailbox.value === 'archive' || mailbox.value === 'snoozed' || mailbox.value === 'aside')
  ) {
    return true;
  }

  if (mailbox.kind === 'section' && mailbox.value === 'inbox') {
    return thread.workflowState === 'inbox' || thread.workflowState === 'todo';
  }

  return (
    thread.workflowState !== 'done'
    && thread.workflowState !== 'snoozed'
    && thread.workflowState !== 'aside'
  );
}
