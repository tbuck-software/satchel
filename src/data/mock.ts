import type {
  BackendCalendarEvent,
  BackendCalendarInvite,
  BackendCalendarSource,
  BackendDraft,
  BackendOutboxItem,
  BackendTopic,
  BackendTopicTimelineItem,
} from '@/services/backend';
import type { Account, Message, Thread } from '@/types/mail';
import { datetimeFormats } from '@/i18n/formats';

export const mockAccounts: Account[] = [
  {
    id: 'acc-work',
    displayName: 'Work',
    email: 'tom@satchel.dev',
    provider: 'imap',
    kind: 'email',
    color: '#55a6ff',
  },
];

/** Yesterday at a fixed clock time, so the fixture lands in Yesterday whenever it runs. */
function yesterdayAt(hours: number, minutes: number): number {
  const day = new Date();
  day.setHours(0, 0, 0, 0);
  day.setDate(day.getDate() - 1);
  day.setHours(hours, minutes);
  return day.getTime();
}

/*
 * Personal mail from people Tom has written to before: each thread holds his
 * earlier message and their answer. These are the mails the smart Inbox lists
 * as plain rows of their day — not Priority, not a bundle, not a new sender.
 */
const knownCorrespondents = [
  {
    id: 'th-30',
    subject: 'Release notes for 4.2',
    sender: { name: 'Lena Vogel', email: 'lena.vogel@app.io' },
    snippet: 'I added the migration notes you asked for. Could you read the upgrade section once more?',
    at: Date.now() - 35 * 60_000,
    unread: true,
    asked: 'Could you add the migration notes to the 4.2 release notes?',
  },
  {
    id: 'th-31',
    subject: 'Floor plan for the new office',
    sender: { name: 'Marco Rossi', email: 'marco@studio-rossi.example' },
    snippet: 'Here is the revised plan with the quiet room by the window, as discussed.',
    at: Date.now() - 95 * 60_000,
    unread: false,
    asked: 'Could the quiet room move to the window side?',
  },
  {
    id: 'th-32',
    subject: 'Lunch on Friday?',
    sender: { name: 'Priya Nair', email: 'priya.nair@app.io' },
    snippet: 'Friday works. The place around the corner at 12:30?',
    at: Date.now() - 140 * 60_000,
    unread: false,
    asked: 'Lunch on Friday? I owe you one for the review.',
  },
  {
    id: 'th-33',
    subject: 'Contract draft, second round',
    sender: { name: 'Daniel Becker', email: 'daniel@becker-law.example' },
    snippet: 'Clause 7 now limits the liability as agreed. Two small open points remain.',
    at: yesterdayAt(16, 40),
    unread: false,
    asked: 'Thanks for the first draft. Clause 7 still needs the liability cap.',
  },
  {
    id: 'th-34',
    subject: 'Retro notes and action items',
    sender: { name: 'Ana Costa', email: 'ana.costa@app.io' },
    snippet: 'Three action items came out of the retro; the first one is yours.',
    at: yesterdayAt(11, 15),
    unread: false,
    asked: 'Could you send the retro notes around when they are ready?',
  },
];

// `messageCount` is the number of messages seeded for the thread below, drafts
// included, as the backend's projection counts them. A thread without seeded
// messages gets one, so its count is 1.
const mockThreadSeeds: Omit<Thread, 'lastMessageAtMs'>[] = [
  {
    id: 'th-1',
    accountId: 'acc-work',
    subject: 'Staging login 500 after deployment',
    participants: [
      { name: 'Sarah Kim', email: 'sarah@app.io' },
      { name: 'Tom', email: 'tom@satchel.dev' },
    ],
    snippet: 'Users cannot sign in on staging since this morning...',
    lastMessageAt: new Date(Date.now() - 20 * 60_000).toISOString(),
    messageCount: 3,
    unreadCount: 1,
    hasAttachments: true,
    workflowState: 'inbox',
    priority: 3,
    mailboxNames: ['INBOX'],
    tags: ['bug', 'staging'],
    dueDate: new Date(Date.now() + 4 * 60 * 60_000).toISOString(),
  },
  {
    id: 'th-2',
    accountId: 'acc-work',
    subject: 'Invoice March (waiting for vendor correction)',
    participants: [
      { name: 'Vendor Ops', email: 'billing@vendor.example' },
      { name: 'Tom', email: 'tom@satchel.dev' },
    ],
    snippet: 'Please confirm the corrected line items in the attached PDF.',
    lastMessageAt: new Date(Date.now() - 26 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: true,
    workflowState: 'snoozed',
    priority: 2,
    mailboxNames: ['INBOX'],
    tags: ['finance'],
    snoozeUntil: new Date(Date.now() + 24 * 60 * 60_000).toISOString(),
  },
  {
    id: 'th-3',
    accountId: 'acc-work',
    subject: 'Team standup notes',
    participants: [{ name: 'Engineering', email: 'eng@satchel.dev' }],
    snippet: 'Decisions from today: cache invalidation, release train...',
    lastMessageAt: new Date(Date.now() - 3 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: ['team'],
  },
  {
    id: 'th-4',
    accountId: 'acc-work',
    subject: 'Conference invitation and CFP reminder',
    participants: [{ name: 'Events', email: 'events@conf.dev' }],
    snippet: 'Last call for papers is this Friday.',
    lastMessageAt: new Date(Date.now() - 8 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['events'],
  },
  {
    id: 'th-5',
    accountId: 'acc-work',
    subject: 'Onboarding plan for new hire',
    participants: [
      { name: 'HR', email: 'hr@satchel.dev' },
      { name: 'Tom', email: 'tom@satchel.dev' },
    ],
    snippet: 'Can you review the first-week checklist?',
    lastMessageAt: new Date(Date.now() - 50 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'done',
    priority: 1,
    mailboxNames: ['Archive'],
    tags: ['hr'],
  },
  {
    id: 'th-6',
    accountId: 'acc-work',
    subject: 'Google Cloud billing threshold reached',
    participants: [{ name: 'Google Cloud', email: 'billing-noreply@cloud.google.com' }],
    snippet: 'Your configured billing threshold for Project Atlas has been reached.',
    lastMessageAt: new Date(Date.now() - 42 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 2,
    mailboxNames: ['INBOX'],
    tags: ['cloud'],
    category: 'updates',
    isStarred: true,
  },
  {
    id: 'th-7',
    accountId: 'acc-work',
    subject: 'Google AI Studio quota warning',
    participants: [{ name: 'Google AI Studio', email: 'no-reply@aistudio.google.com' }],
    snippet: 'Your usage is approaching the daily quota for Gemini API calls.',
    lastMessageAt: new Date(Date.now() - 48 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 2,
    mailboxNames: ['INBOX'],
    tags: ['ai'],
    category: 'updates',
    isStarred: true,
  },
  {
    id: 'th-8',
    accountId: 'acc-work',
    subject: 'Google Maps Platform key restriction reminder',
    participants: [{ name: 'Google Maps Platform', email: 'maps-noreply@google.com' }],
    snippet: 'Protect production keys by adding HTTP referrer restrictions.',
    lastMessageAt: new Date(Date.now() - 55 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: ['maps'],
    category: 'updates',
    isStarred: true,
  },
  {
    id: 'th-9',
    accountId: 'acc-work',
    subject: 'App Store Connect: Build 42 finished processing',
    participants: [{ name: 'App Store Connect', email: 'no_reply@email.apple.com' }],
    snippet: 'Version 0.3.0 build 42 is ready for TestFlight review.',
    lastMessageAt: new Date(Date.now() - 65 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: ['release'],
    category: 'updates',
  },
  {
    id: 'th-10',
    accountId: 'acc-work',
    subject: 'GitHub Actions: deploy.yml failed',
    participants: [{ name: 'GitHub', email: 'notifications@github.com' }],
    snippet: 'The deploy workflow failed on main after 3m 12s.',
    lastMessageAt: new Date(Date.now() - 72 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 2,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 2,
    mailboxNames: ['INBOX'],
    tags: ['ci'],
    category: 'updates',
  },
  {
    id: 'th-10b',
    accountId: 'acc-work',
    subject: 'GitHub Actions: deploy.yml failed',
    participants: [{ name: 'GitHub', email: 'notifications@github.com' }],
    snippet: 'The deploy workflow failed on main after 2m 41s.',
    lastMessageAt: new Date(Date.now() - 75 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 2,
    mailboxNames: ['INBOX'],
    tags: ['ci'],
    category: 'updates',
  },
  {
    id: 'th-10c',
    accountId: 'acc-work',
    subject: 'Ihre Rechnung für Juni ist verfügbar',
    participants: [{ name: 'Telekom Rechnung Online', email: 'rechnungonline@telekom.de' }],
    snippet: 'Die Rechnung für Ihren Festnetz-Anschluss liegt im Kundencenter bereit.',
    lastMessageAt: new Date(Date.now() - 26 * 3_600_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: true,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: [],
    category: 'updates',
  },
  {
    id: 'th-10d',
    accountId: 'acc-work',
    subject: 'Backup-Report der letzten Woche',
    participants: [{ name: 'Restic Monitor', email: 'backup@satchel.dev' }],
    snippet: 'Alle 7 Snapshots wurden erfolgreich erstellt, 0 Fehler.',
    lastMessageAt: new Date(Date.now() - 8 * 86_400_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: [],
    category: 'updates',
  },
  {
    id: 'th-11',
    accountId: 'acc-work',
    subject: 'Sentry alert: API error rate above threshold',
    participants: [{ name: 'Sentry', email: 'alerts@sentry.io' }],
    snippet: 'Error rate for api.satchel.dev crossed the configured threshold.',
    lastMessageAt: new Date(Date.now() - 86 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 2,
    mailboxNames: ['INBOX'],
    tags: ['alert'],
    category: 'updates',
  },
  {
    id: 'th-12',
    accountId: 'acc-work',
    subject: 'Linear: Inbox triage milestone moved',
    participants: [{ name: 'Linear', email: 'notifications@linear.app' }],
    snippet: 'The Inbox triage milestone was moved to next Friday.',
    lastMessageAt: new Date(Date.now() - 97 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: ['planning'],
    category: 'updates',
  },
  {
    id: 'th-13',
    accountId: 'acc-work',
    subject: 'Stripe payout is on the way',
    participants: [{ name: 'Stripe', email: 'support@stripe.com' }],
    snippet: 'A payout of EUR 4,218.90 is expected to arrive tomorrow.',
    lastMessageAt: new Date(Date.now() - 110 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: ['finance'],
    category: 'updates',
  },
  {
    id: 'th-14',
    accountId: 'acc-work',
    subject: 'DHL Paket wurde zugestellt',
    participants: [{ name: 'DHL Paket', email: 'paket@dhl.de' }],
    snippet: 'Ihre Sendung wurde an den gewuenschten Ablageort geliefert.',
    lastMessageAt: new Date(Date.now() - 128 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['delivery'],
    category: 'updates',
  },
  {
    id: 'th-15',
    accountId: 'acc-work',
    subject: 'Amazon Spar-Abo wird bald versendet',
    participants: [{ name: 'Amazon.de', email: 'auto-confirm@amazon.de' }],
    snippet: 'Prüfen Sie Ihre Spar-Abo Lieferung vor dem Versand.',
    lastMessageAt: new Date(Date.now() - 140 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['shopping'],
    category: 'updates',
  },
  {
    id: 'th-25',
    accountId: 'acc-work',
    subject: 'Expedia Group Lodging Team: Booking update',
    participants: [{ name: 'Expedia Group Lodging Team', email: 'lodging@expedia.com' }],
    snippet: 'A partner message requires review before the end of the day.',
    lastMessageAt: new Date(Date.now() - 150 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['travel'],
    category: 'updates',
  },
  {
    id: 'th-26',
    accountId: 'acc-work',
    subject: 'Google Business Profile: Neue Rezension',
    participants: [{ name: 'Google Business Profile', email: 'businessprofile-noreply@google.com' }],
    snippet: 'Sie haben eine neue Rezension erhalten, die beantwortet werden kann.',
    lastMessageAt: new Date(Date.now() - 160 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['business'],
    category: 'updates',
  },
  {
    id: 'th-27',
    accountId: 'acc-work',
    subject: 'Vercel deployment succeeded',
    participants: [{ name: 'Vercel', email: 'notifications@vercel.com' }],
    snippet: 'Production deployment for satchel-web completed successfully.',
    lastMessageAt: new Date(Date.now() - 170 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['deploy'],
    category: 'updates',
  },
  {
    id: 'th-16',
    accountId: 'acc-work',
    subject: 'METRO Wochenangebote',
    participants: [{ name: 'METRO Wochenangebote', email: 'newsletter@metro.de' }],
    snippet: 'Neue Angebote für Büro, Küche und Vorrat sind verfügbar.',
    lastMessageAt: new Date(Date.now() - 3 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['offers'],
    category: 'promotions',
  },
  {
    id: 'th-17',
    accountId: 'acc-work',
    subject: 'TechCrunch Daily: Funding rounds and AI launches',
    participants: [{ name: 'TechCrunch via LinkedIn', email: 'news@linkedin.com' }],
    snippet: 'Today in startups: new AI launches, acquisitions and funding rounds.',
    lastMessageAt: new Date(Date.now() - 3.3 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['news'],
    category: 'promotions',
  },
  {
    id: 'th-18',
    accountId: 'acc-work',
    subject: 'Kickstarter News Track: Projects ending soon',
    participants: [{ name: 'Kickstarter News Track', email: 'news@kickstarter.com' }],
    snippet: 'Three saved projects are ending within the next 48 hours.',
    lastMessageAt: new Date(Date.now() - 3.7 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['news'],
    category: 'promotions',
  },
  {
    id: 'th-19',
    accountId: 'acc-work',
    subject: 'Product Hunt Daily Digest',
    participants: [{ name: 'Product Hunt', email: 'hello@producthunt.com' }],
    snippet: 'The most discussed tools from today, including new developer workflows.',
    lastMessageAt: new Date(Date.now() - 4.1 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['news'],
    category: 'promotions',
  },
  {
    id: 'th-20',
    accountId: 'acc-work',
    subject: 'The Pragmatic Engineer: Platform teams',
    participants: [{ name: 'The Pragmatic Engineer', email: 'newsletter@pragmaticengineer.com' }],
    snippet: 'A deep dive on platform teams and developer productivity.',
    lastMessageAt: new Date(Date.now() - 4.6 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['newsletter'],
    category: 'promotions',
  },
  {
    id: 'th-21',
    accountId: 'acc-work',
    subject: 'Hacker News Weekly',
    participants: [{ name: 'Hacker News Weekly', email: 'weekly@hackernewsletter.com' }],
    snippet: 'Top engineering discussions and releases from this week.',
    lastMessageAt: new Date(Date.now() - 5 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['newsletter'],
    category: 'promotions',
  },
  {
    id: 'th-22',
    accountId: 'acc-work',
    subject: 'LinkedIn: 4 updates from your network',
    participants: [{ name: 'LinkedIn', email: 'messages-noreply@linkedin.com' }],
    snippet: 'New comments, profile views and messages from your network.',
    lastMessageAt: new Date(Date.now() - 5.4 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['social'],
    category: 'social',
  },
  {
    id: 'th-23',
    accountId: 'acc-work',
    subject: 'X: Login from a new device',
    participants: [{ name: 'X', email: 'info@x.com' }],
    snippet: 'We noticed a login from a new device. Review this activity.',
    lastMessageAt: new Date(Date.now() - 5.8 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: ['social'],
    category: 'social',
  },
  {
    id: 'th-24',
    accountId: 'acc-work',
    subject: 'Coffee with design systems group',
    participants: [{ name: 'Mina Patel', email: 'mina@design.example' }],
    snippet: 'Could we move coffee to Thursday? I want to bring the new component audit.',
    lastMessageAt: new Date(Date.now() - 6.2 * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 1,
    mailboxNames: ['INBOX'],
    tags: ['personal'],
    category: 'personal',
  },
  /*
   * Spam from people nobody has written to: the unknown-sender review must
   * stay in the Inbox and not offer to allow them here.
   */
  ...[
    { id: 'th-spam-1', subject: 'You have won a voucher', name: 'Prize Desk', email: 'claims@prize-desk.example', hours: 3 },
    { id: 'th-spam-2', subject: 'Invoice overdue — act now', name: 'Billing Center', email: 'billing@invoice-center.example', hours: 9 },
    { id: 'th-spam-3', subject: 'Exclusive investment opportunity', name: 'Mr. Adebayo', email: 'adebayo@capital-trust.example', hours: 30 },
  ].map(({ id, subject, name, email, hours }): Omit<Thread, 'lastMessageAtMs'> => ({
    id,
    accountId: 'acc-work',
    subject,
    participants: [{ name, email }],
    snippet: 'Click the link below to confirm your details within 24 hours.',
    lastMessageAt: new Date(Date.now() - hours * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 1,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['Spam'],
    tags: [],
    category: 'personal',
  })),
  /*
   * Mail already checked off: in Done the primary action sends it back to the
   * Inbox instead of offering Done again.
   */
  ...[
    { id: 'th-done-1', subject: 'Signed contract is in the shared drive', name: 'Jonas Weber', email: 'jonas.weber@app.io', hours: 4 },
    { id: 'th-done-2', subject: 'Thanks for the quick review', name: 'Lena Vogel', email: 'lena.vogel@app.io', hours: 27 },
  ].map(({ id, subject, name, email, hours }): Omit<Thread, 'lastMessageAtMs'> => ({
    id,
    accountId: 'acc-work',
    subject,
    participants: [{ name, email }, { name: 'Tom', email: 'tom@satchel.dev' }],
    snippet: 'All set from my side, nothing more to do here.',
    lastMessageAt: new Date(Date.now() - hours * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'done',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: [],
    category: 'personal',
    senderKnown: true,
  })),
  /*
   * Custom folders with names as long as real ones get: the folder browser
   * has to keep its row actions inside each row at every width.
   */
  ...[
    { id: 'th-folder-1', folder: 'Abuse - Meldungen und Beschwerden aus dem Kundenportal', hours: 50 },
    { id: 'th-folder-2', folder: 'Projekte/Kunden/Müller & Söhne Werkzeugmaschinen GmbH & Co. KG – Wartungsvertrag', hours: 70 },
    { id: 'th-folder-3', folder: 'Archiv/2025', hours: 90 },
  ].map(({ id, folder, hours }): Omit<Thread, 'lastMessageAtMs'> => ({
    id,
    accountId: 'acc-work',
    subject: 'Ticket update',
    participants: [{ name: 'Support', email: 'support@portal.example' }],
    snippet: 'The ticket was updated.',
    lastMessageAt: new Date(Date.now() - hours * 60 * 60_000).toISOString(),
    messageCount: 1,
    unreadCount: 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: [folder],
    tags: [],
    category: 'updates',
    senderKnown: true,
  })),
  ...knownCorrespondents.map(({ id, subject, sender, snippet, at, unread }): Omit<Thread, 'lastMessageAtMs'> => ({
    id,
    accountId: 'acc-work',
    subject,
    participants: [sender, { name: 'Tom', email: 'tom@satchel.dev' }],
    snippet,
    lastMessageAt: new Date(at).toISOString(),
    messageCount: 2,
    unreadCount: unread ? 1 : 0,
    hasAttachments: false,
    workflowState: 'inbox',
    priority: 0,
    mailboxNames: ['INBOX'],
    tags: [],
    category: 'personal',
    // The core sets this when the thread holds a message we sent: the
    // correspondence below is what makes these senders known.
    senderKnown: true,
  })),
];

export const mockThreads: Thread[] = mockThreadSeeds.map((seed) => ({
  ...seed,
  lastMessageAtMs: Date.parse(seed.lastMessageAt),
}));

/** The words of the thread draft, shown on its card and resumed by Reply. */
const OFFER_TEXT = 'Der OAuth-Fix ist auf Staging. Kannst du den Login noch einmal gegenprüfen?';

const baseMessages: Message[] = [
  {
    id: 'm-1',
    threadId: 'th-1',
    from: { name: 'Sarah Kim', email: 'sarah@app.io' },
    to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
    subject: 'Staging login 500 after deployment',
    bodyText:
      'Users cannot sign in on staging since this morning. Steps: open staging login, enter valid credentials, click sign in, receive HTTP 500.',
    timestamp: new Date(Date.now() - 120 * 60_000).toISOString(),
    isRead: true,
    attachments: [{ id: 'a-1', filename: 'error-log.txt', contentType: 'text/plain', size: 12_338 }],
  },
  {
    id: 'm-2',
    threadId: 'th-1',
    from: { name: 'Tom', email: 'tom@satchel.dev' },
    to: [{ name: 'Sarah Kim', email: 'sarah@app.io' }],
    subject: 'Re: Staging login 500 after deployment',
    bodyText: 'Looking into it. Seems related to an OAuth config change. Update in 30 min.',
    timestamp: new Date(Date.now() - 100 * 60_000).toISOString(),
    isRead: true,
    // One picture and one document on the newest message: the attachment card
    // draws a thumbnail for the first and a type badge for the second, and
    // both paths were unreachable in the mock while they were unattached.
    attachments: [
      { id: 'a-2', filename: 'staging-login.png', contentType: 'image/png', size: 48_210 },
      { id: 'a-3', filename: 'oauth-config.pdf', contentType: 'application/pdf', size: 184_000 },
    ],
  },
  {
    id: 'dr-offer',
    threadId: 'th-1',
    from: { name: 'Tom', email: 'tom@satchel.dev' },
    to: [{ name: 'Sarah Kim', email: 'sarah@app.io' }],
    subject: 'Re: Staging login 500 after deployment',
    bodyText: OFFER_TEXT,
    timestamp: new Date(Date.now() - 25 * 60_000).toISOString(),
    isRead: true,
    isDraft: true,
    attachments: [],
  },
  {
    id: 'm-3',
    threadId: 'th-2',
    from: { name: 'Vendor Ops', email: 'billing@vendor.example' },
    to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
    subject: 'Invoice March (waiting for vendor correction)',
    bodyText: 'Please confirm line item 4 and tax percentage once reviewed.',
    timestamp: new Date(Date.now() - 28 * 60 * 60_000).toISOString(),
    isRead: true,
    attachments: [{ id: 'a-4', filename: 'invoice-march.pdf', contentType: 'application/pdf', size: 1_200_000 }],
  },
  {
    id: 'm-4',
    threadId: 'th-3',
    from: { name: 'Engineering', email: 'eng@satchel.dev' },
    to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
    subject: 'Team standup notes',
    bodyText: 'Highlights: cache invalidation fix merged, release postponed by one day.',
    timestamp: new Date(Date.now() - 3 * 60 * 60_000).toISOString(),
    isRead: false,
    attachments: [],
  },
  {
    // HTML, so the reader renders it in the sandboxed frame: the WebKit
    // keyboard smoke test clicks into this body.
    id: 'm-th-7',
    threadId: 'th-7',
    from: { name: 'Google AI Studio', email: 'no-reply@aistudio.google.com' },
    to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
    subject: 'Google AI Studio quota warning',
    bodyText: 'Your usage is approaching the daily quota for Gemini API calls.',
    bodyHtml: '<p>Your usage is approaching the daily quota for Gemini API calls.</p>'
      + '<ul><li>Requests today: 1,840 of 2,000</li><li>Quota resets at midnight UTC</li></ul>',
    timestamp: new Date(Date.now() - 48 * 60_000).toISOString(),
    isRead: true,
    attachments: [],
  },
];

/*
 * Two designed HTML mails as senders ship them: fixed desktop-width tables,
 * px font sizes with px line heights, multi-column grids, Outlook's MsoNormal
 * paragraphs. The phone reader has to keep these legible; the plain fixtures
 * above never exercise that.
 */
const NEWSLETTER_HTML = `<html><head><style>
  .hero-title { font-family: Arial, sans-serif; font-size: 30px; line-height: 32px; font-weight: bold; color: #003d7c; }
  .cell { font-family: Arial, sans-serif; font-size: 14px; line-height: 17px; color: #333333; }
  .price { font-family: Arial, sans-serif; font-size: 22px; line-height: 24px; font-weight: bold; color: #e30613; }
  .small { font-family: Arial, sans-serif; font-size: 11px; line-height: 13px; color: #777777; }
</style></head><body style="margin:0;padding:0;background:#f2f2f2">
<table width="600" cellpadding="0" cellspacing="0" border="0" align="center" bgcolor="#ffffff" style="width:600px">
  <tr><td style="padding:16px 24px;background:#003d7c"><span style="font-family:Arial,sans-serif;font-size:20px;line-height:22px;color:#ffe600;font-weight:bold">METRO</span>
    <span style="font-family:Arial,sans-serif;font-size:11px;line-height:12px;color:#ffffff;float:right">Im Browser ansehen</span></td></tr>
  <tr><td style="padding:24px 24px 8px" class="hero-title">Wochenangebote für Büro, Küche und Vorrat</td></tr>
  <tr><td style="padding:0 24px 16px" class="cell">Gültig von Montag, 26.10. bis Samstag, 31.10. in allen Märkten und im Onlineshop. Solange der Vorrat reicht.</td></tr>
  <tr><td style="padding:0 24px 16px"><a href="https://www.metro.de/angebote" style="display:inline-block;background:#e30613;color:#ffffff;font-family:Arial,sans-serif;font-size:15px;line-height:18px;font-weight:bold;padding:12px 28px;text-decoration:none;border-radius:3px">Alle Angebote ansehen</a></td></tr>
  <tr><td style="padding:0 16px 16px"><table width="568" cellpadding="0" cellspacing="0" border="0"><tr>
    <td width="284" valign="top" style="padding:8px" class="cell"><img src="https://img.metro.de/kaffee.jpg" width="268" height="160" alt="Kaffeebohnen 1 kg" style="display:block;background:#e8e2d6"><br><b>Kaffee Crema, ganze Bohne, 1 kg</b><br>Kräftig und mild geröstet, für Vollautomaten.<br><span class="price">12,99 €</span><br><span class="small">1 kg = 12,99 € · zzgl. MwSt.</span></td>
    <td width="284" valign="top" style="padding:8px" class="cell"><img src="https://img.metro.de/papier.jpg" width="268" height="160" alt="Kopierpapier A4" style="display:block;background:#e8e2d6"><br><b>Kopierpapier A4, 80 g/m², 2500 Blatt</b><br>Für Laser- und Tintenstrahldrucker.<br><span class="price">19,49 €</span><br><span class="small">1000 Blatt = 7,80 € · zzgl. MwSt.</span></td>
  </tr></table></td></tr>
  <tr><td style="padding:16px 24px;background:#f2f2f2" class="small">Sie erhalten diesen Newsletter, weil Sie sich mit tom@satchel.dev angemeldet haben. <a href="https://www.metro.de/abmelden" style="color:#003d7c">Abmelden</a> · <a href="https://www.metro.de/datenschutz" style="color:#003d7c">Datenschutz</a><br>METRO Deutschland GmbH · Metro-Straße 1 · 40235 Düsseldorf</td></tr>
</table></body></html>`;

const DIGEST_HTML = `<html><head><style>
  p.MsoNormal { margin: 0cm; font-size: 11.0pt; font-family: Calibri, sans-serif; }
  .headline { font-size: 17px; line-height: 19px; font-weight: 600; color: #0a66c2; margin: 0; }
  .meta { font-size: 12px; line-height: 14px; color: #666666; margin: 0; }
</style></head><body style="margin:0">
<table width="640" cellpadding="0" cellspacing="0" style="width:640px;font-family:-apple-system,Helvetica,Arial,sans-serif">
  <tr><td style="padding:20px 24px 4px"><p class="MsoNormal" style="font-size:24px;line-height:26px;font-weight:700">TechCrunch Daily</p></td></tr>
  <tr><td style="padding:0 24px 16px"><p class="meta">Tuesday, October 27 · 4 stories · 6 min read</p></td></tr>
  <tr><td style="padding:12px 24px;border-top:1px solid #e0e0e0"><p class="headline">A payments startup raises $40M to take on card fees in Europe</p><p class="meta">Funding · 2 min</p></td></tr>
  <tr><td style="padding:12px 24px;border-top:1px solid #e0e0e0"><p class="headline">The new on-device models are fast enough to replace cloud calls for mail triage</p><p class="meta">AI · 3 min</p></td></tr>
  <tr><td style="padding:12px 24px;border-top:1px solid #e0e0e0"><p class="headline">Why local-first apps are back</p><p class="meta">Opinion · 4 min</p></td></tr>
  <tr><td style="padding:16px 24px"><p class="MsoNormal">You are receiving this digest because you follow TechCrunch on LinkedIn.</p><p class="MsoNormal" style="font-size:9pt;line-height:10pt;color:#999999">LinkedIn Corporation, 1000 W Maude Ave, Sunnyvale, CA 94085. <a href="https://www.linkedin.com/unsubscribe">Unsubscribe</a></p></td></tr>
</table></body></html>`;

baseMessages.push(
  {
    id: 'm-th-16', threadId: 'th-16',
    from: { name: 'METRO Wochenangebote', email: 'newsletter@metro.de' },
    to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
    subject: 'METRO Wochenangebote',
    bodyText: 'Neue Angebote für Büro, Küche und Vorrat sind verfügbar.',
    bodyHtml: NEWSLETTER_HTML,
    // A real newsletter carries the header, so the reader offers the one-click
    // unsubscribe here as it does in the app.
    listUnsubscribe: '<https://www.metro.de/abmelden>',
    listUnsubscribePost: 'List-Unsubscribe=One-Click',
    timestamp: new Date(Date.now() - 3 * 60 * 60_000).toISOString(),
    isRead: false, attachments: [],
  },
  {
    id: 'm-th-17', threadId: 'th-17',
    from: { name: 'TechCrunch via LinkedIn', email: 'news@linkedin.com' },
    to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
    subject: 'TechCrunch Daily: Funding rounds and AI launches',
    bodyText: 'Today in startups: new AI launches, acquisitions and funding rounds.',
    bodyHtml: DIGEST_HTML,
    timestamp: new Date(Date.now() - 3.3 * 60 * 60_000).toISOString(),
    isRead: true, attachments: [],
  },
);

// Tom's message first, then the answer that brought the thread back.
baseMessages.push(...knownCorrespondents.flatMap(({ id, subject, sender, snippet, at, unread, asked }) => [
  {
    id: `m-${id}-sent`,
    threadId: id,
    from: { name: 'Tom', email: 'tom@satchel.dev' },
    to: [sender],
    subject,
    bodyText: asked,
    timestamp: new Date(at - 3 * 60 * 60_000).toISOString(),
    isRead: true,
    attachments: [],
  },
  {
    id: `m-${id}`,
    threadId: id,
    from: sender,
    to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
    subject: `Re: ${subject}`,
    bodyText: `Hi Tom,\n\n${snippet}\n\n${sender.name.split(' ')[0]}`,
    timestamp: new Date(at).toISOString(),
    isRead: !unread,
    attachments: [],
  },
]));

const seededMessagesByThread = baseMessages.reduce<Record<string, Message[]>>((acc, message) => {
  const list = acc[message.threadId] ?? [];
  list.push(message);
  acc[message.threadId] = list;
  return acc;
}, {});

for (const thread of mockThreads) {
  if (seededMessagesByThread[thread.id]) {
    continue;
  }

  seededMessagesByThread[thread.id] = [
    {
      id: `m-${thread.id}`,
      threadId: thread.id,
      from: thread.participants[0] ?? { name: "Tom", email: "tom@satchel.dev" },
      to: [{ name: 'Tom', email: 'tom@satchel.dev' }],
      subject: thread.subject,
      bodyText: thread.snippet,
      timestamp: thread.lastMessageAt,
      isRead: thread.unreadCount === 0,
      attachments: [],
    },
  ];
}

export const mockMessagesByThread = seededMessagesByThread;

/** One outgoing mail per queue status, so the Outbox can be seen without a live account. */
export const mockOutboxItems: BackendOutboxItem[] = [
  {
    id: 'failed-move', account_id: 'acc-1', action_type: 'remote_mutation', status: 'failed',
    mutation_kind: 'workflow_move',
    subject: 'Project archive', recipients: [], retry_count: 6,
    created_at: new Date(Date.now() - 3600000).toISOString(),
    updated_at: new Date(Date.now() - 600000).toISOString(),
    last_error: 'Target folder no longer exists on the server',
  },
  {
    id: 'ob-failed',
    account_id: 'acc-work',
    action_type: 'send_message',
    status: 'failed',
    retry_count: 1,
    created_at: new Date(Date.now() - 14 * 60_000).toISOString(),
    updated_at: new Date(Date.now() - 2 * 60_000).toISOString(),
    last_error: 'gmail send skipped: rate limited for another 730s',
    next_retry_at: new Date(Date.now() + 12 * 60_000).toISOString(),
    scheduled_for: null,
    subject: 'Angebot Websiterelaunch',
    recipients: ['mail@example.com'],
  },
  {
    id: 'ob-sending',
    account_id: 'acc-work',
    action_type: 'send_message',
    status: 'sending',
    retry_count: 0,
    created_at: new Date(Date.now() - 20_000).toISOString(),
    updated_at: new Date(Date.now() - 5_000).toISOString(),
    last_error: null,
    next_retry_at: null,
    scheduled_for: null,
    subject: 'Re: Staging login 500 after deployment',
    recipients: ['sarah@app.io', 'ops@app.io', 'team@app.io'],
  },
  {
    id: 'ob-scheduled',
    account_id: 'acc-work',
    action_type: 'send_message',
    status: 'scheduled',
    retry_count: 0,
    created_at: new Date(Date.now() - 3 * 3_600_000).toISOString(),
    updated_at: new Date(Date.now() - 3 * 3_600_000).toISOString(),
    last_error: null,
    next_retry_at: null,
    scheduled_for: new Date(Date.now() + 15 * 3_600_000).toISOString(),
    subject: 'Wochenrückblick KW 31',
    recipients: ['team@app.io'],
  },
];

/**
 * The thread draft as the composer saves a reply: the answer, then the quoted
 * conversation newest first, the signature as metadata beside the body and the
 * message it answers in its send options. Resuming it has to bring back all
 * of that, not only the words.
 */
const offerQuotes = baseMessages
  .filter((message) => message.threadId === 'th-1' && !message.isDraft)
  .reverse()
  .map((message) => ({
    // The intro the composer writes (`compose.reply.intro`, `formatTimestamp`).
    intro: `On ${new Date(message.timestamp).toLocaleString('en', { ...datetimeFormats.en!.dateTime, year: 'numeric' })}, ${message.from.name} <${message.from.email}> wrote:`,
    lines: (message.bodyText ?? '').split('\n'),
  }));
const escapeHtml = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const offerDraftBody = OFFER_TEXT + offerQuotes
  .map((quote) => `\n\n${quote.intro}\n${quote.lines.map((line) => `> ${line}`).join('\n')}`)
  .join('');
const offerDraftHtml = `<p>${escapeHtml(OFFER_TEXT)}</p>` + offerQuotes
  .map((quote) => `<blockquote>${[quote.intro, ...quote.lines].map((line) => `<p>${escapeHtml(line)}</p>`).join('')}</blockquote>`)
  .join('');

/** One scheduled and two ordinary drafts, so both groups are visible. */
export const mockDrafts: BackendDraft[] = [
  {
    id: 'dr-scheduled',
    account_id: 'acc-work',
    to: ['team@app.io'],
    cc: [],
    bcc: [],
    subject: 'Wochenrückblick KW 31',
    body: 'Kurzer Überblick über die Woche: Staging steht wieder, der Release ist auf Freitag geschoben.',
    scheduled_for: new Date(Date.now() + 15 * 3_600_000).toISOString(),
    updated_at: new Date(Date.now() - 2 * 3_600_000).toISOString(),
  },
  {
    id: 'dr-offer',
    account_id: 'acc-work',
    thread_id: 'th-1',
    to: ['sarah@app.io'],
    cc: [],
    bcc: [],
    subject: 'Re: Staging login 500 after deployment',
    body: offerDraftBody,
    body_html: offerDraftHtml,
    // Resolves with `?mockSignatures=1`; without it the account has none.
    signature_id: 'signature-acc-work-0',
    signature_variant: 'auto',
    send_options: { reply_to_message_id: 'm-1' },
    updated_at: new Date(Date.now() - 25 * 60_000).toISOString(),
  },
  {
    id: 'dr-empty',
    account_id: 'acc-work',
    to: [],
    cc: [],
    bcc: [],
    subject: '',
    body: '',
    updated_at: new Date(Date.now() - 4 * 60_000).toISOString(),
  },
];

/** One topic with a mixed feed, so the issue view can be seen without a live account. */
export const mockTopics: BackendTopic[] = [
  {
    id: 'topic-mock-1',
    name: 'Staging-Ausfall Juli',
    description: 'Alles rund um den Login-Ausfall auf Staging: Mails, Zwischenstände aus Chats, Entscheidungen.',
    color: '#3b82f6',
    icon: null,
    status: 'active',
    due_date: null,
    item_count: 2,
    note_count: 2,
    unread_count: 1,
    last_sender_name: 'Sarah Kim',
    last_sender_email: 'sarah@app.io',
    created_at: new Date(Date.now() - 3 * 24 * 60 * 60_000).toISOString(),
    updated_at: new Date(Date.now() - 60 * 60_000).toISOString(),
  },
];

export const mockTopicTimeline: Record<string, BackendTopicTimelineItem[]> = {
  'topic-mock-1': [
    {
      item_type: 'note',
      item_id: 'note-mock-2',
      thread_id: null,
      title: null,
      snippet: 'Telefonat mit Sarah: Rollback um 15 Uhr, danach Postmortem.',
      content: 'Telefonat mit Sarah: Rollback um 15 Uhr, danach Postmortem.',
      from_address: null,
      sort_date: new Date(Date.now() - 60 * 60_000).toISOString(),
      added_at: new Date(Date.now() - 60 * 60_000).toISOString(),
      added_by: 'user',
      message_count: 0,
      unread_count: 0,
    },
    {
      item_type: 'thread',
      item_id: 'th-1',
      thread_id: 'th-1',
      title: 'Staging login 500 after deployment',
      snippet: 'Users cannot sign in on staging since this morning...',
      content: null,
      from_address: 'sarah@app.io',
      sort_date: new Date(Date.now() - 20 * 60_000).toISOString(),
      added_at: new Date(Date.now() - 2 * 60 * 60_000).toISOString(),
      added_by: 'user',
      message_count: 4,
      unread_count: 1,
    },
    {
      item_type: 'note',
      item_id: 'note-mock-1',
      thread_id: null,
      title: null,
      snippet: 'Aus Discord (#ops): OAuth-Config wurde gestern geändert.',
      content: 'Aus Discord (#ops): OAuth-Config wurde gestern geändert — vermutlich die Ursache.',
      from_address: null,
      sort_date: new Date(Date.now() - 5 * 60 * 60_000).toISOString(),
      added_at: new Date(Date.now() - 5 * 60 * 60_000).toISOString(),
      added_by: 'user',
      message_count: 0,
      unread_count: 0,
    },
  ],
};

/*
 * A day with something in it, so the sidebar's Today card and the calendar
 * have anything to draw. All-day entries, one with a long title, because those
 * are the rows that have no time to print and whose label has to give way —
 * and three of them, because that is what crowded the timed events out of
 * the Today card. Around 14:00 a short appointment starts just before a long
 * one with a half-hour meeting inside it: the overlap the week grid has to
 * lay out side by side.
 */
const startOfToday = () => {
  const day = new Date();
  day.setHours(0, 0, 0, 0);
  return day;
};

function localDate(at: Date): string {
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

function mockEventAt(
  id: string,
  title: string,
  startHour: number | null,
  durationMinutes: number,
): BackendCalendarEvent {
  const base = startOfToday();
  const start = startHour === null ? base : new Date(base.getTime() + startHour * 3_600_000);
  const end = startHour === null
    ? new Date(base.getTime() + 24 * 3_600_000)
    : new Date(start.getTime() + durationMinutes * 60_000);
  // An all-day event is a calendar date, as Google and CalDAV store it. The
  // UTC instant of local midnight is the day before east of Greenwich.
  const boundary = (at: Date) => startHour === null ? localDate(at) : at.toISOString();
  return {
    id,
    account_id: 'acc-work',
    uid: `${id}@satchel.dev`,
    title,
    description: null,
    location: null,
    starts_at: boundary(start),
    ends_at: boundary(end),
    all_day: startHour === null ? 1 : 0,
    attendees_json: '[]',
    source: 'local',
    collection_href: null,
    created_at: start.toISOString(),
    updated_at: start.toISOString(),
  };
}

export const mockCalendarEvents: BackendCalendarEvent[] = [
  mockEventAt('ev-all-day', 'Weltkindertag (Thüringen) — landesweiter Feiertag', null, 0),
  mockEventAt('ev-cure', 'Ole in Kur', null, 0),
  mockEventAt('ev-cooking', 'Kochtag', null, 0),
  mockEventAt('ev-podiatry', 'Podologie', 13.75, 45),
  mockEventAt('ev-childcare', '🎈🎈 Kinderbetreuung Herbstferien', 14, 120),
  mockEventAt('ev-coffee', 'code.example coffee', 14, 30),
  mockEventAt('ev-client', 'Client call · Mike', 16.5, 45),
  mockEventAt('ev-review', 'Sprint review', 11, 60),
  mockEventAt('ev-retro', 'Retro', 17.5, 45),
];

function mockInvite(
  id: string,
  fields: Partial<BackendCalendarInvite> & Pick<BackendCalendarInvite, 'method'>,
): BackendCalendarInvite {
  const created = startOfToday().toISOString();
  return {
    id,
    message_id: null,
    account_id: 'acc-work',
    uid: `${id}@satchel.dev`,
    organizer_email: 'lena@example.com',
    organizer_name: 'Lena Hoffmann',
    summary: null,
    description: null,
    location: null,
    starts_at: null,
    ends_at: null,
    all_day: 0,
    rsvp_status: 'needs-action',
    raw_ics: 'BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n',
    sequence: 0,
    cancelled: 0,
    recurrence_id: null,
    recurring: 0,
    recurrence_unbounded: 0,
    created_at: created,
    updated_at: created,
    ...fields,
  };
}

const tomorrowAt = (hour: number) => new Date(startOfToday().getTime() + (24 + hour) * 3_600_000).toISOString();

/*
 * One invitation to answer, and one answer to an invitation: the METHOD:REPLY
 * another account sent back after accepting. It has no title and no time,
 * and the list must not offer it as something to accept.
 */
export const mockCalendarInvites: BackendCalendarInvite[] = [
  mockInvite('inv-architecture', {
    method: 'REQUEST',
    summary: 'Architecture review',
    starts_at: tomorrowAt(10),
    ends_at: tomorrowAt(11),
  }),
  mockInvite('inv-reply', { method: 'REPLY', organizer_email: 'me@satchel.dev', organizer_name: null }),
];

export const mockCalendarSources: BackendCalendarSource[] = [
  { account_id: 'acc-work', source: 'local', remote_id: 'local', name: 'Satchel' },
];
