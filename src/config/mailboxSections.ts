import type { WorkflowFilter } from '@/types/workflow';

export type SectionIcon =
  | 'inbox'
  | 'drafts'
  | 'sent'
  | 'outbox'
  | 'trash'
  | 'archive'
  | 'clock'
  | 'aside'
  | 'spam'
  | 'bell'
  | 'blocked';

export type CounterMode = 'unread' | 'threads' | 'none';
export type SectionSource = 'mailbox' | 'workflow' | 'route';

export type SectionSpec = {
  id: string;
  labelKey: string;
  /** One sentence on what the view is for, where views are easy to confuse. */
  hintKey?: string;
  icon: SectionIcon;
  source: SectionSource;
  counterMode: CounterMode;
  mailboxType?: string;
  workflowState?: Exclude<WorkflowFilter, 'all'>;
  routeName?: string;
  forceVisible?: boolean;
  defaultOpen?: boolean;
};

/** The sidebar's fixed sections, in order. The palette offers each one as a view. */
export const MAILBOX_SECTIONS: readonly SectionSpec[] = [
  {
    id: 'inbox',
    labelKey: 'nav.folders.inbox',
    icon: 'inbox',
    source: 'mailbox',
    mailboxType: 'inbox',
    counterMode: 'unread',
    forceVisible: true,
    defaultOpen: true,
  },
  {
    // Snoozed mail and mail waiting for a reply: both come back on their own.
    id: 'later',
    labelKey: 'nav.folders.later',
    hintKey: 'inbox.empty.later.description',
    icon: 'clock',
    source: 'workflow',
    workflowState: 'later',
    // No counter, as for Done: only loaded threads could be counted, and
    // waiting mail leaves the loaded Inbox page the moment it is filed.
    counterMode: 'none',
    forceVisible: true,
  },
  {
    id: 'aside',
    labelKey: 'nav.folders.aside',
    hintKey: 'inbox.empty.aside.description',
    icon: 'aside',
    source: 'workflow',
    workflowState: 'aside',
    counterMode: 'threads',
    forceVisible: true,
  },
  {
    id: 'outbox',
    labelKey: 'nav.folders.outbox',
    icon: 'outbox',
    source: 'route',
    routeName: 'outbox',
    counterMode: 'threads',
    forceVisible: true,
  },
  {
    id: 'archive',
    labelKey: 'nav.folders.done',
    icon: 'archive',
    source: 'workflow',
    workflowState: 'done',
    // No counter: workflow sections can only count threads that happen to be
    // loaded, so the number showed "checked off this session", not a mailbox
    // stat. forceVisible keeps the section from popping in with the first
    // archived mail — the sidebar must not move on a check-off.
    counterMode: 'none',
    forceVisible: true,
    defaultOpen: true,
  },
  {
    id: 'drafts',
    labelKey: 'nav.folders.drafts',
    icon: 'drafts',
    source: 'mailbox',
    mailboxType: 'drafts',
    counterMode: 'threads',
  },
  {
    id: 'sent',
    labelKey: 'nav.folders.sent',
    icon: 'sent',
    source: 'mailbox',
    mailboxType: 'sent',
    counterMode: 'none',
    defaultOpen: true,
  },
  {
    id: 'trash',
    labelKey: 'nav.folders.trash',
    icon: 'trash',
    source: 'mailbox',
    mailboxType: 'trash',
    counterMode: 'none',
  },
  {
    id: 'spam',
    labelKey: 'nav.folders.spam',
    icon: 'spam',
    source: 'mailbox',
    mailboxType: 'spam',
    counterMode: 'unread',
  },
  {
    id: 'notifications',
    labelKey: 'nav.folders.notifications',
    icon: 'bell',
    source: 'mailbox',
    mailboxType: 'notifications',
    counterMode: 'unread',
  },
  {
    id: 'blocked',
    labelKey: 'nav.folders.blocked',
    icon: 'blocked',
    source: 'mailbox',
    mailboxType: 'blocked',
    counterMode: 'unread',
  },
];
