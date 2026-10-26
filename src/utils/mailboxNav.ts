import type { WorkflowState } from '@/types/workflow';
import type { MailboxFilter } from '@/utils/threadList';

/**
 * Routes that show the thread list the sidebar tree scopes. Anywhere else —
 * search, compose, follow-ups — the list filter is only remembered state, and
 * marking Inbox as the current place would be wrong.
 */
const MAILBOX_ROUTES = new Set(['inbox', 'thread']);

export interface MailboxNavState {
  routeName: string | null;
  workflowFilter: WorkflowState | 'all';
  mailboxFilter: MailboxFilter;
  activeAccountId: string | null;
}

export function sectionIdForWorkflow(state: WorkflowState): string {
  if (state === 'done') return 'archive';
  if (state === 'snoozed') return 'snoozed';
  return state;
}

/** The tree node that is the current place, or '' when the route is outside the tree. */
export function activeMailboxNavNode(state: MailboxNavState): string {
  const { routeName, workflowFilter, mailboxFilter, activeAccountId } = state;
  if (routeName === 'outbox') return 'section:outbox';
  if (!routeName || !MAILBOX_ROUTES.has(routeName)) return '';

  if (workflowFilter !== 'all' && mailboxFilter.kind === 'all') {
    return `section:${sectionIdForWorkflow(workflowFilter)}`;
  }

  if (workflowFilter === 'all' && mailboxFilter.kind === 'section' && mailboxFilter.value) {
    return `section:${mailboxFilter.value}`;
  }

  if (workflowFilter === 'all' && mailboxFilter.kind === 'folder' && mailboxFilter.value) {
    return activeAccountId
      ? `folder:${activeAccountId}:${mailboxFilter.value}`
      : `folder:${mailboxFilter.value}`;
  }

  return '';
}
