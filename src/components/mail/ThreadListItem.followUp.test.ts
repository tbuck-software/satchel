// @vitest-environment jsdom
import { nextTick } from 'vue';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { useToast } from '@/composables/useToast';

const backend = vi.hoisted(() => ({
  backendUpsertFollowUp: vi.fn(),
  backendListFollowUps: vi.fn(async () => []),
  backendCancelFollowUp: vi.fn(async () => {}),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
}));

vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  ...backend,
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({
    archive: vi.fn(),
    setAside: vi.fn(),
    moveToInbox: vi.fn(),
    snooze: vi.fn(),
    snoozeAt: vi.fn(),
    setPinned: vi.fn(),
    trash: vi.fn(),
    setRead: vi.fn(),
  }),
}));

import ThreadListItem from './ThreadListItem.vue';

const DUE = '2026-10-01T08:00:00.000Z';

/** Stands in for the async SnoozePicker: one button that picks a date. */
const PickerStub = {
  emits: ['choose', 'close'],
  template: `<button data-test="pick" @click="$emit('choose', '${DUE}')">pick</button>`,
};

const thread = {
  id: 'thread-1',
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
  isStarred: false,
};

function mountRow() {
  return mount(ThreadListItem, {
    attachTo: document.body,
    props: { thread, active: false, selected: false, selectionMode: false },
    global: { stubs: { SnoozePicker: PickerStub } },
  });
}

async function openFollowUpPicker() {
  const wrapper = mountRow();
  await wrapper.get('article').trigger('contextmenu', { clientX: 20, clientY: 30 });
  await nextTick();

  const entry = Array.from(document.body.querySelectorAll<HTMLButtonElement>('.context-menu button'))
    .find((button) => button.textContent?.trim() === 'Follow up …');
  expect(entry, 'the row context menu offers a follow-up entry').toBeDefined();
  entry!.click();
  await nextTick();
  return wrapper;
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  useToast().clear();
  window.localStorage.clear();
});

describe('scheduling a follow-up from the thread row', () => {
  it('sends the picked date to the backend and confirms it', async () => {
    backend.backendUpsertFollowUp.mockResolvedValue({
      id: 'f-1',
      account_id: 'account',
      thread_id: 'thread-1',
      due_at: DUE,
      status: 'pending',
      created_at: DUE,
      updated_at: DUE,
    });

    const wrapper = await openFollowUpPicker();
    await wrapper.get('[data-test="pick"]').trigger('click');
    await vi.waitFor(() => expect(backend.backendUpsertFollowUp).toHaveBeenCalledTimes(1));

    expect(backend.backendUpsertFollowUp).toHaveBeenCalledWith({
      id: null,
      threadId: 'thread-1',
      dueAt: DUE,
      note: null,
    });
    await vi.waitFor(() =>
      expect(useToast().state.toasts[0]?.message).toContain('Follow-up set for'),
    );
    wrapper.unmount();
  });

  it('reports a refused reminder instead of pretending it was saved', async () => {
    backend.backendUpsertFollowUp.mockRejectedValue(new Error('database is locked'));

    const wrapper = await openFollowUpPicker();
    await wrapper.get('[data-test="pick"]').trigger('click');

    await vi.waitFor(() => {
      const toast = useToast().state.toasts[0];
      expect(toast?.tone).toBe('error');
      expect(toast?.message).toContain('database is locked');
    });
    wrapper.unmount();
  });

  it('does not reach the backend before a date is picked', async () => {
    const wrapper = await openFollowUpPicker();

    expect(backend.backendUpsertFollowUp).not.toHaveBeenCalled();
    wrapper.unmount();
  });
});
