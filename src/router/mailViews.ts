import type { Router } from 'vue-router';
import { useThreadsStore } from '@/stores/threads';
import type { WorkflowFilter } from '@/types/workflow';
import type { MailboxFilter } from '@/utils/threadList';

/**
 * The mail lists share one route (`inbox`); which list it shows is thread
 * store state. These are the lists with a name of their own, so the sidebar,
 * the `g` keys, the palette and the redirect routes all land on the same
 * scope instead of each setting the filters its own way.
 */
export type MailListView = 'inbox' | 'later' | 'aside' | 'done' | 'sent';

/** Views that are a list scope rather than a page; each also has a redirect route. */
export const MAIL_LIST_VIEWS: readonly MailListView[] = ['inbox', 'later', 'aside', 'done', 'sent'];
/** Mailbox sections among them; the rest are workflow states. */
const SECTIONS: ReadonlySet<string> = new Set(['inbox', 'sent']);

/** Point the shared list at a scope. */
export function showMailList(workflow: WorkflowFilter, mailbox: MailboxFilter) {
  const threads = useThreadsStore();
  threads.workflowFilter = workflow;
  threads.setMailboxFilter(mailbox);
}

export function selectMailListView(view: MailListView) {
  if (SECTIONS.has(view)) showMailList('all', { kind: 'section', value: view });
  else showMailList(view as WorkflowFilter, { kind: 'all', value: null });
}

/** Open a view by route name: a list scope on the inbox route, any other page as itself. */
export function openView(router: Router, name: string) {
  if ((MAIL_LIST_VIEWS as readonly string[]).includes(name)) {
    selectMailListView(name as MailListView);
    return router.push({ name: 'inbox' });
  }
  return router.push({ name });
}
