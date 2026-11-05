// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import type { Thread } from '@/types/mail';

const actions = vi.hoisted(() => ({
  markNotSpam: vi.fn(async () => {}),
}));

vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendListFollowUps: vi.fn(async () => []),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({
    toggleDone: vi.fn(),
    allDone: () => false,
    setAside: vi.fn(),
    moveToInbox: vi.fn(),
    setPinned: vi.fn(),
    trash: vi.fn(),
    setRead: vi.fn(),
    markNotSpam: actions.markNotSpam,
  }),
}));

import ThreadListItem from './ThreadListItem.vue';

function thread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 'thread-1',
    accountId: 'account',
    subject: 'Claim your prize',
    participants: [{ name: 'Lottery', email: 'win@lottery.example' }],
    snippet: 'You won',
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
    ...overrides,
  };
}

function mountRow(value: Thread) {
  return mount(ThreadListItem, {
    attachTo: document.body,
    props: { thread: value, active: false, selected: false, selectionMode: false },
    global: { stubs: { SnoozePicker: true } },
  });
}

async function menuEntries(wrapper: ReturnType<typeof mountRow>) {
  await wrapper.get('article').trigger('contextmenu', { clientX: 20, clientY: 30 });
  await vi.waitFor(() => expect(document.body.querySelector('.context-menu')).not.toBeNull());
  return Array.from(document.body.querySelectorAll<HTMLButtonElement>('.context-menu button'));
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  document.body.innerHTML = '';
});

describe('likely spam in the thread list', () => {
  it('names the verdict in words on the row', () => {
    const wrapper = mountRow(thread({ likelySpam: true }));
    expect(wrapper.find('.spam-label').text()).toBe('Likely spam');
    wrapper.unmount();
  });

  it('shows no mark on ordinary mail', () => {
    const wrapper = mountRow(thread());
    expect(wrapper.find('.spam-label').exists()).toBe(false);
    wrapper.unmount();
  });

  it('offers Not spam in the row menu and runs it for the row', async () => {
    const wrapper = mountRow(thread({ likelySpam: true }));
    const entry = (await menuEntries(wrapper)).find((button) => button.textContent?.trim() === 'Not spam');
    expect(entry).toBeDefined();
    entry!.click();
    await vi.waitFor(() => expect(actions.markNotSpam).toHaveBeenCalledWith(['thread-1']));
    wrapper.unmount();
  });

  it('offers Not spam for mail sitting in a Spam folder', async () => {
    const wrapper = mountRow(thread({ mailboxNames: ['Junk'] }));
    const labels = (await menuEntries(wrapper)).map((button) => button.textContent?.trim());
    expect(labels).toContain('Not spam');
    wrapper.unmount();
  });

  it('keeps Not spam out of the menu of ordinary Inbox mail', async () => {
    const wrapper = mountRow(thread());
    const labels = (await menuEntries(wrapper)).map((button) => button.textContent?.trim());
    expect(labels).not.toContain('Not spam');
    wrapper.unmount();
  });
});
