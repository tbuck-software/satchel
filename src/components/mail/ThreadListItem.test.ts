// @vitest-environment jsdom
import { nextTick } from 'vue';
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, shallowMount } from '@vue/test-utils';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import ThreadListItem from './ThreadListItem.vue';

/** The row's menus are async components: wait until `count` of them are in the DOM. */
async function contextMenusOpen(count = 1) {
  await vi.waitFor(() => expect(document.body.querySelectorAll('.context-menu')).toHaveLength(count));
}

const { archiveMock, setAsideMock, moveToInboxMock, setPinnedMock, trashMock } = vi.hoisted(() => ({
  archiveMock: vi.fn(),
  setAsideMock: vi.fn(),
  moveToInboxMock: vi.fn(),
  setPinnedMock: vi.fn(),
  trashMock: vi.fn(),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({
    archive: archiveMock,
    setAside: setAsideMock,
    moveToInbox: moveToInboxMock,
    snooze: vi.fn(),
    setPinned: setPinnedMock,
    trash: trashMock,
    setRead: (ids: string[], read: boolean) => useThreadsStore().setThreadsRead(ids, read),
  }),
}));

beforeEach(() => {
  setActivePinia(createPinia());
  archiveMock.mockReset();
  setAsideMock.mockReset();
  moveToInboxMock.mockReset();
  setPinnedMock.mockReset();
  trashMock.mockReset();
  window.localStorage.clear();
});

afterEach(() => vi.useRealTimers());
afterAll(() => vi.dynamicImportSettled());

describe('thread row layout', () => {
  it('lifts the open thread onto a sheet and drops the unread weight', () => {
    const thread = {
      id: 'thread',
      accountId: 'account',
      subject: 'Betreff',
      participants: [{ name: 'Absender', email: 'sender@example.com' }],
      snippet: 'Vorschau',
      lastMessageAt: '2026-08-06T12:00:00Z',
      lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
      messageCount: 1,
      unreadCount: 0,
      hasAttachments: false,
      workflowState: 'inbox' as const,
      priority: 1 as const,
      mailboxNames: ['INBOX'],
      tags: [],
    };
    const wrapper = shallowMount(ThreadListItem, {
      props: { thread, active: true, selected: false, selectionMode: false },
    });

    expect(wrapper.get('[data-swipe-surface]').classes()).toContain('thread-row__surface--open');
    expect(wrapper.html()).not.toContain('font-[650]');
  });

  it('checks the row from its avatar and opens it from anywhere else', async () => {
    const thread = {
      id: 'thread', accountId: 'account', subject: 'Betreff',
      participants: [{ name: 'Absender', email: 'sender@example.com' }], snippet: 'Vorschau',
      lastMessageAt: '2026-08-06T12:00:00Z', lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
      messageCount: 1, unreadCount: 0, hasAttachments: false, workflowState: 'inbox' as const,
      priority: 1 as const, mailboxNames: ['INBOX'], tags: [],
    };
    const wrapper = shallowMount(ThreadListItem, {
      props: { thread, active: false, selected: false, selectionMode: false },
    });

    await wrapper.get('[data-test="thread-row-check"]').trigger('click', { shiftKey: true });
    expect(wrapper.emitted('check')).toEqual([[{ threadId: 'thread', shiftKey: true, metaKey: false, ctrlKey: false }]]);
    expect(wrapper.emitted('activate')).toBeUndefined();

    await wrapper.get('article').trigger('click');
    expect(wrapper.emitted('activate')).toHaveLength(1);
  });

  it('draws the mute and attachment icons at full tertiary, not faded below 3:1', () => {
    const thread = {
      id: 'thread',
      accountId: 'account',
      subject: 'Betreff',
      participants: [{ name: 'Absender', email: 'sender@example.com' }],
      snippet: 'Vorschau',
      lastMessageAt: '2026-08-06T12:00:00Z',
      lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
      messageCount: 1,
      unreadCount: 0,
      hasAttachments: true,
      isMuted: true,
      workflowState: 'inbox' as const,
      priority: 1 as const,
      mailboxNames: ['INBOX'],
      tags: [],
    };
    const wrapper = shallowMount(ThreadListItem, {
      props: { thread, active: false, selected: false, selectionMode: false },
    });
    const icons = wrapper.findAll('svg.text-tertiary');

    expect(icons.length).toBeGreaterThanOrEqual(2);
    for (const icon of icons) expect(icon.classes().filter((name) => name.startsWith('opacity-'))).toEqual([]);
  });

  it('forwards list-wrapper sizing classes to the article when using a teleported menu', () => {
    const wrapper = shallowMount(ThreadListItem, {
      attrs: { class: 'w-full' },
      props: {
        thread: {
          id: 'thread',
          accountId: 'account',
          subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau',
          lastMessageAt: '2026-08-06T12:00:00Z',
          lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
          messageCount: 1,
          unreadCount: 0,
          hasAttachments: false,
          workflowState: 'inbox',
          priority: 1,
          mailboxNames: ['INBOX'],
          tags: [],
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });

    expect(wrapper.get('article').classes()).toContain('w-full');
  });

  it('only renders the strong focus treatment for keyboard navigation', () => {
    const thread = {
      id: 'thread',
      accountId: 'account',
      subject: 'Betreff',
      participants: [{ name: 'Absender', email: 'sender@example.com' }],
      snippet: 'Vorschau',
      lastMessageAt: '2026-08-06T12:00:00Z',
      lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
      messageCount: 1,
      unreadCount: 0,
      hasAttachments: false,
      workflowState: 'inbox' as const,
      priority: 1 as const,
      mailboxNames: ['INBOX'],
      tags: [],
    };
    const wrapper = shallowMount(ThreadListItem, {
      props: {
        thread,
        active: true,
        selected: false,
        selectionMode: false,
      },
    });

    expect(wrapper.get('[data-swipe-surface]').classes())
      .not.toContain('thread-row__surface--keyboard-focused');

    void wrapper.setProps({ keyboardFocused: true });
    return nextTick().then(() => {
      expect(wrapper.get('[data-swipe-surface]').classes())
        .toContain('thread-row__surface--keyboard-focused');
    });
  });

  it('switches from the short to the long action at the second swipe threshold', async () => {
    const wrapper = mount(ThreadListItem, {
      props: {
        thread: {
          id: 'thread',
          accountId: 'account',
          subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau',
          lastMessageAt: '2026-08-06T12:00:00Z',
          lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
          messageCount: 1,
          unreadCount: 0,
          hasAttachments: false,
          workflowState: 'inbox',
          priority: 1,
          mailboxNames: ['INBOX'],
          tags: [],
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });

    const row = wrapper.get('article');
    const surface = wrapper.get('[data-swipe-surface]');
    expect(wrapper.find('[data-swipe-action]').exists()).toBe(false);

    await row.trigger('pointerdown', {
      pointerId: 1,
      pointerType: 'touch',
      clientX: 20,
      clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 1,
      pointerType: 'touch',
      clientX: 58,
      clientY: 42,
    });
    expect(surface.attributes('style')).toContain('translate3d(38px');
    expect(wrapper.get('[data-swipe-action="toggle-read"]').text()).toContain('Unread');
    expect(wrapper.get('[data-swipe-action="toggle-read"]').classes()).not.toContain('is-armed');

    await row.trigger('pointermove', {
      pointerId: 1,
      pointerType: 'touch',
      clientX: 112,
      clientY: 42,
    });
    expect(wrapper.get('[data-swipe-action="toggle-read"]').classes()).toContain('is-armed');

    await row.trigger('pointermove', {
      pointerId: 1,
      pointerType: 'touch',
      clientX: 188,
      clientY: 42,
    });
    expect(wrapper.get('[data-swipe-action="done"]').text()).toContain('Done');
    expect(wrapper.get('[data-swipe-action="done"]').attributes('data-swipe-slot')).toBe('rightLong');
    expect(wrapper.get('[data-swipe-action="done"]').classes()).toContain('is-armed');

    await row.trigger('pointercancel', { pointerId: 1, pointerType: 'touch' });
    expect(surface.attributes('style')).toContain('translate3d(0px');

    await row.trigger('pointerdown', {
      pointerId: 2,
      pointerType: 'touch',
      clientX: 120,
      clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 2,
      pointerType: 'touch',
      clientX: 40,
      clientY: 42,
    });
    expect(surface.attributes('style')).toContain('translate3d(-80px');
    expect(wrapper.get('[data-swipe-action="pin"]').text()).toContain('Pin');
    expect(wrapper.get('[data-swipe-action="pin"]').classes()).toContain('is-armed');

    await row.trigger('pointermove', {
      pointerId: 2,
      pointerType: 'touch',
      clientX: -48,
      clientY: 42,
    });
    expect(wrapper.get('[data-swipe-action="trash"]').text()).toContain('Delete');
    expect(wrapper.get('[data-swipe-action="trash"]').attributes('data-swipe-slot')).toBe('leftLong');
  });

  it('commits a horizontal swipe but yields a vertical gesture to scrolling', async () => {
    vi.useFakeTimers();
    const wrapper = mount(ThreadListItem, {
      props: {
        thread: {
          id: 'thread', accountId: 'account', subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau', lastMessageAt: '2026-08-06T12:00:00Z', lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'), messageCount: 1,
          unreadCount: 0, hasAttachments: false, workflowState: 'inbox', priority: 1,
          mailboxNames: ['INBOX'], tags: [],
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });
    const row = wrapper.get('article');

    await row.trigger('pointerdown', {
      pointerId: 3, pointerType: 'touch', clientX: 20, clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 3, pointerType: 'touch', clientX: 24, clientY: 80,
    });
    await row.trigger('pointerup', {
      pointerId: 3, pointerType: 'touch', clientX: 24, clientY: 80,
    });
    expect(archiveMock).not.toHaveBeenCalled();
    expect(wrapper.get('[data-swipe-surface]').attributes('style')).toContain('translate3d(0px');

    await row.trigger('pointerdown', {
      pointerId: 4, pointerType: 'touch', clientX: 20, clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 4, pointerType: 'touch', clientX: 190, clientY: 42,
    });
    await row.trigger('pointerup', {
      pointerId: 4, pointerType: 'touch', clientX: 190, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);
    expect(archiveMock).toHaveBeenCalledWith(['thread']);
    expect(setAsideMock).not.toHaveBeenCalled();
  });

  it('uses the configured action for a short swipe', async () => {
    vi.useFakeTimers();
    useSettingsStore().setSwipeAction('rightShort', 'aside');
    const wrapper = mount(ThreadListItem, {
      props: {
        thread: {
          id: 'thread', accountId: 'account', subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau', lastMessageAt: '2026-08-06T12:00:00Z', lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'), messageCount: 1,
          unreadCount: 0, hasAttachments: false, workflowState: 'inbox', priority: 1,
          mailboxNames: ['INBOX'], tags: [],
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });
    const row = wrapper.get('article');

    await row.trigger('pointerdown', {
      pointerId: 5, pointerType: 'touch', clientX: 20, clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 5, pointerType: 'touch', clientX: 108, clientY: 42,
    });
    expect(wrapper.get('[data-swipe-action="aside"]').text()).toContain('Aside');
    await row.trigger('pointerup', {
      pointerId: 5, pointerType: 'touch', clientX: 108, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);

    expect(setAsideMock).toHaveBeenCalledWith(['thread']);
  });

  it('marks a read thread unread with the default short right swipe', async () => {
    vi.useFakeTimers();
    const threads = useThreadsStore();
    const setThreadsRead = vi.spyOn(threads, 'setThreadsRead').mockResolvedValue();
    const wrapper = mount(ThreadListItem, {
      props: {
        thread: {
          id: 'thread', accountId: 'account', subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau', lastMessageAt: '2026-08-06T12:00:00Z', lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'), messageCount: 1,
          unreadCount: 0, hasAttachments: false, workflowState: 'inbox', priority: 1,
          mailboxNames: ['INBOX'], tags: [],
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });
    const row = wrapper.get('article');

    await row.trigger('pointerdown', {
      pointerId: 6, pointerType: 'touch', clientX: 20, clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 6, pointerType: 'touch', clientX: 108, clientY: 42,
    });
    await row.trigger('pointerup', {
      pointerId: 6, pointerType: 'touch', clientX: 108, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);

    expect(setThreadsRead).toHaveBeenCalledWith(['thread'], false);
  });

  it('commits pin and delete from the two left swipe distances', async () => {
    vi.useFakeTimers();
    const wrapper = mount(ThreadListItem, {
      props: {
        thread: {
          id: 'thread', accountId: 'account', subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau', lastMessageAt: '2026-08-06T12:00:00Z', lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'), messageCount: 1,
          unreadCount: 0, hasAttachments: false, workflowState: 'inbox', priority: 1,
          mailboxNames: ['INBOX'], tags: [], isStarred: false,
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });
    const row = wrapper.get('article');

    await row.trigger('pointerdown', {
      pointerId: 7, pointerType: 'touch', clientX: 200, clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 7, pointerType: 'touch', clientX: 112, clientY: 42,
    });
    await row.trigger('pointerup', {
      pointerId: 7, pointerType: 'touch', clientX: 112, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);
    expect(setPinnedMock).toHaveBeenCalledWith(['thread'], true);

    await row.trigger('pointerdown', {
      pointerId: 8, pointerType: 'touch', clientX: 200, clientY: 40,
    });
    await row.trigger('pointermove', {
      pointerId: 8, pointerType: 'touch', clientX: 30, clientY: 42,
    });
    await row.trigger('pointerup', {
      pointerId: 8, pointerType: 'touch', clientX: 30, clientY: 42,
    });
    await vi.advanceTimersByTimeAsync(120);
    expect(trashMock).toHaveBeenCalledWith(['thread']);
  });

  it('offers the same core filing actions from the row context menu', async () => {
    const wrapper = mount(ThreadListItem, {
      attachTo: document.body,
      props: {
        thread: {
          id: 'thread',
          accountId: 'account',
          subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau',
          lastMessageAt: '2026-08-06T12:00:00Z',
          lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
          messageCount: 1,
          unreadCount: 0,
          hasAttachments: false,
          workflowState: 'inbox',
          priority: 1,
          mailboxNames: ['INBOX'],
          tags: [],
          isStarred: false,
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });

    expect(wrapper.find('button[title="Pin"]').exists()).toBe(false);
    expect(wrapper.find('button[title="Set aside"]').exists()).toBe(false);

    await wrapper.get('article').trigger('contextmenu', { clientX: 20, clientY: 30 });
    await contextMenusOpen();

    expect(document.activeElement).toBe(document.body.querySelector('.context-menu button'));
    const actions = Array.from(document.body.querySelectorAll('.context-menu button'))
      .map((button) => button.textContent?.trim() ?? '');
    expect(actions).toEqual(expect.arrayContaining([
      'Pin',
      'Set aside',
      'Snooze',
      'Done',
      'Mark as unread',
    ]));
    expect(document.body.textContent).not.toContain('Star');
    expect(document.body.textContent).not.toContain('Favourite');

    useThreadsStore().selectedThreadIds = ['thread', 'another-thread'];
    const done = Array.from(document.body.querySelectorAll<HTMLButtonElement>('.context-menu button'))
      .find(button => button.textContent?.trim() === 'Done');
    done!.click();
    await nextTick();
    expect(archiveMock).toHaveBeenLastCalledWith(['thread', 'another-thread']);

    wrapper.unmount();
  });

  it('names only the action the current state allows, never both halves', async () => {
    const wrapper = mount(ThreadListItem, {
      attachTo: document.body,
      props: {
        thread: {
          id: 'thread',
          accountId: 'account',
          subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau',
          lastMessageAt: '2026-08-06T12:00:00Z',
          lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
          messageCount: 1,
          unreadCount: 0,
          hasAttachments: false,
          workflowState: 'aside',
          priority: 2,
          mailboxNames: ['INBOX'],
          tags: [],
          isStarred: true,
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });

    await wrapper.get('article').trigger('contextmenu', { clientX: 20, clientY: 30 });
    await contextMenusOpen();

    const menu = document.body.querySelector('.context-menu');
    const actions = Array.from(menu?.querySelectorAll('button') ?? [])
      .map((button) => button.textContent?.trim() ?? '');

    expect(actions).toContain('Remove pin');
    expect(actions).not.toContain('Pin');
    expect(actions).toContain('Back to the inbox');
    expect(actions).not.toContain('Set aside');

    // Priority is one row whose checkmark carries the state — the old menu
    // listed setting and clearing it side by side.
    const priorityRows = actions.filter((label) => label.startsWith('Priority'));
    expect(priorityRows).toEqual(['Priority']);
    expect(menu?.querySelector('[aria-checked="true"]')?.textContent?.trim()).toBe('Priority');

    wrapper.unmount();
  });

  it('files the sender from a flyout instead of six top-level rows', async () => {
    const wrapper = mount(ThreadListItem, {
      attachTo: document.body,
      global: { stubs: { SenderFilingMenu: { template: '<i data-test="sender-menu" />' } } },
      props: {
        thread: {
          id: 'thread',
          accountId: 'account',
          subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau',
          lastMessageAt: '2026-08-06T12:00:00Z',
          lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
          messageCount: 1,
          unreadCount: 0,
          hasAttachments: false,
          workflowState: 'inbox',
          priority: 1,
          mailboxNames: ['INBOX'],
          tags: [],
          isStarred: false,
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });

    await wrapper.get('article').trigger('contextmenu', { clientX: 20, clientY: 30 });
    await contextMenusOpen();

    expect(document.body.textContent).not.toContain('Newsletter');
    expect(document.body.querySelector('[data-test="sender-menu"]')).toBeNull();

    const trigger = Array.from(document.body.querySelectorAll<HTMLButtonElement>('.context-menu button'))
      .find((button) => button.textContent?.trim() === 'Sort sender into…');
    expect(trigger?.getAttribute('aria-haspopup')).toBe('menu');
    trigger?.click();
    await contextMenusOpen(2);

    expect(document.body.querySelector('[data-test="sender-menu"]')).not.toBeNull();

    wrapper.unmount();
  });

  it('mounts its menus only once they are first opened', async () => {
    const wrapper = mount(ThreadListItem, {
      attachTo: document.body,
      global: { stubs: { SenderFilingMenu: { template: '<i data-test="sender-menu" />' } } },
      props: {
        thread: {
          id: 'thread',
          accountId: 'account',
          subject: 'Betreff',
          participants: [{ name: 'Absender', email: 'sender@example.com' }],
          snippet: 'Vorschau',
          lastMessageAt: '2026-08-06T12:00:00Z',
          lastMessageAtMs: Date.parse('2026-08-06T12:00:00Z'),
          messageCount: 1,
          unreadCount: 0,
          hasAttachments: false,
          workflowState: 'inbox',
          priority: 1,
          mailboxNames: ['INBOX'],
          tags: [],
          isStarred: false,
        },
        active: false,
        selected: false,
        selectionMode: false,
      },
    });

    expect(wrapper.findAllComponents({ name: 'ContextMenu' })).toHaveLength(0);

    await wrapper.get('article').trigger('contextmenu', { clientX: 20, clientY: 30 });
    await contextMenusOpen();
    expect(wrapper.findAllComponents({ name: 'ContextMenu' })).toHaveLength(1);
    expect(document.body.querySelector('.context-menu')).not.toBeNull();

    const trigger = Array.from(document.body.querySelectorAll<HTMLButtonElement>('.context-menu button'))
      .find((button) => button.textContent?.trim() === 'Sort sender into…');
    trigger?.click();
    await contextMenusOpen(2);
    expect(wrapper.findAllComponents({ name: 'ContextMenu' })).toHaveLength(2);

    wrapper.unmount();
  });
});
