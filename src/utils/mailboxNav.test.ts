import { describe, expect, it } from 'vitest';
import { activeMailboxNavNode, type MailboxNavState } from './mailboxNav';

const inboxScope: Omit<MailboxNavState, 'routeName'> = {
  workflowFilter: 'all',
  mailboxFilter: { kind: 'section', value: 'inbox' },
  activeAccountId: null,
};

describe('activeMailboxNavNode', () => {
  it('marks the scoped section on the list and thread routes', () => {
    expect(activeMailboxNavNode({ ...inboxScope, routeName: 'inbox' })).toBe('section:inbox');
    expect(activeMailboxNavNode({ ...inboxScope, routeName: 'thread' })).toBe('section:inbox');
  });

  it('marks nothing on routes outside the tree, whatever the list filter remembers', () => {
    for (const routeName of ['search', 'compose', 'followups', 'new-senders', null]) {
      expect(activeMailboxNavNode({ ...inboxScope, routeName })).toBe('');
    }
  });

  it('marks the outbox on its own route', () => {
    expect(activeMailboxNavNode({ ...inboxScope, routeName: 'outbox' })).toBe('section:outbox');
  });

  it('maps workflow filters and folders to their nodes', () => {
    expect(activeMailboxNavNode({
      routeName: 'inbox',
      workflowFilter: 'done',
      mailboxFilter: { kind: 'all', value: null },
      activeAccountId: null,
    })).toBe('section:archive');
    expect(activeMailboxNavNode({
      routeName: 'inbox',
      workflowFilter: 'later',
      mailboxFilter: { kind: 'all', value: null },
      activeAccountId: null,
    })).toBe('section:later');
    expect(activeMailboxNavNode({
      routeName: 'inbox',
      workflowFilter: 'all',
      mailboxFilter: { kind: 'folder', value: 'Projects' },
      activeAccountId: 'acc-1',
    })).toBe('folder:acc-1:Projects');
  });
});
