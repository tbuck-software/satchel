// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  activeTopics: [] as Array<typeof topic>,
  addItemsToTopic: vi.fn(),
  addToTopic: vi.fn(),
  closePicker: vi.fn(),
  create: vi.fn(),
  pickerTarget: { itemType: 'message', itemIds: ['message-1'] } as { itemType: 'message' | 'thread'; itemIds: string[] },
  moveToFolder: vi.fn(),
  focusNeighborThread: vi.fn(),
  threads: {} as Record<string, { accountId: string; mailboxNames: string[] }>,
  mailboxes: [] as Array<{ id: string; name: string; account_id: string }>,
}));

vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => ({
    activeTopics: harness.activeTopics,
    get pickerTarget() { return harness.pickerTarget; },
    addItemsToTopic: harness.addItemsToTopic,
    byId: () => undefined,
    closePicker: harness.closePicker,
    create: harness.create,
    membershipsKnownFor: () => true, membershipsFailedFor: () => false, topicIdsFor: () => [],
  }),
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => ({
    activeThreadId: 'thread-1',
    find: (id: string) => harness.threads[id],
    focusNeighborThread: harness.focusNeighborThread,
  }),
}));

vi.mock('@/stores/mailboxes', () => ({
  useMailboxesStore: () => ({ mailboxes: harness.mailboxes }),
}));

vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ registerOverlay: () => vi.fn() }),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({ addToTopic: harness.addToTopic, removeFromTopic: vi.fn(), moveToFolder: harness.moveToFolder }),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ error: vi.fn(), success: vi.fn() }),
}));

import FilePickerOverlay from './FilePickerOverlay.vue';

const topic = {
  id: 'topic-1',
  name: 'Hunterport Quota Änderung',
  description: '',
  color: null,
  icon: null,
  status: 'active',
  due_date: null,
  item_count: 0,
  note_count: 0,
  unread_count: 0,
  last_sender_name: null,
  last_sender_email: null,
  created_at: '2026-08-11T07:00:00Z',
  updated_at: '2026-08-11T07:00:00Z',
};

function render(attach = false) {
  return mount(FilePickerOverlay, {
    ...(attach ? { attachTo: document.body } : {}),
    global: { stubs: { Teleport: true, Transition: false } },
  });
}

describe('topic creation feedback', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.activeTopics.length = 0;
    harness.addItemsToTopic.mockResolvedValue(undefined);
  });

  it('shows the local topic immediately without blocking and ignores a second activation', async () => {
    let finishCreate!: (value: typeof topic) => void;
    const persisted = new Promise<typeof topic>((resolve) => { finishCreate = resolve; });
    harness.create.mockImplementationOnce(() => {
      harness.activeTopics.push(topic);
      return { topic, persisted };
    });
    const wrapper = render();
    const input = wrapper.get('input');
    await input.setValue(topic.name);
    const createRow = wrapper.get('.row');

    const enterUpdate = input.trigger('keydown', { key: 'Enter' });
    const clickUpdate = createRow.trigger('click');
    await Promise.all([enterUpdate, clickUpdate]);

    expect(harness.create).toHaveBeenCalledTimes(1);
    expect(wrapper.get('.row').text()).toBe(topic.name);
    expect(wrapper.get('.panel').attributes('aria-busy')).toBeUndefined();
    expect(wrapper.get('.row').attributes()).not.toHaveProperty('disabled');
    expect(input.attributes()).not.toHaveProperty('disabled');
    expect(harness.addItemsToTopic).not.toHaveBeenCalled();

    finishCreate(topic);
    await flushPromises();

    expect(harness.addItemsToTopic).toHaveBeenCalledTimes(1);
    expect(harness.addItemsToTopic).toHaveBeenCalledWith(
      topic.id,
      'message',
      ['message-1'],
    );
  });
});

describe('keyboard containment', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.activeTopics.length = 0;
    harness.activeTopics.push(topic);
  });

  /**
   * Without the trap the picker opened over the message list and left the focus
   * in it: Tab walked the mail behind the dialog, and closing dropped the focus
   * on `<body>`.
   */
  it('cycles Tab inside the panel and returns the focus to the opener', async () => {
    const opener = document.createElement('button');
    document.body.append(opener);
    opener.focus();

    const wrapper = render(true);
    const panel = wrapper.get('.panel');
    const input = wrapper.get('input').element as HTMLElement;
    const lastRow = wrapper.findAll('.row').at(-1)!.element as HTMLElement;

    lastRow.focus();
    await panel.trigger('keydown', { key: 'Tab' });
    expect(document.activeElement).toBe(input);

    await panel.trigger('keydown', { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(lastRow);

    wrapper.unmount();
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });
});

describe('folders and topics in one picker', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.activeTopics.length = 0;
    harness.activeTopics.push(topic);
    harness.pickerTarget = { itemType: 'thread', itemIds: ['thread-1'] };
    harness.threads = { 'thread-1': { accountId: 'acc', mailboxNames: ['INBOX'] } };
    harness.mailboxes = [
      { id: 'f-inbox', name: 'INBOX', account_id: 'acc' },
      { id: 'f-projects', name: 'Projects', account_id: 'acc' },
      { id: 'f-other', name: 'Elsewhere', account_id: 'other' },
    ];
  });

  it('lists the other folders of the account above the topics', () => {
    const wrapper = render();

    expect(wrapper.findAll('.group-label').map((label) => label.text())).toEqual(['Folders', 'Topics']);
    expect(wrapper.findAll('.row').map((row) => row.text())).toEqual(['Projects', topic.name]);
  });

  it('moves the mail and closes when a folder is chosen', async () => {
    const wrapper = render();

    await wrapper.findAll('.row')[0]!.trigger('click');

    expect(harness.closePicker).toHaveBeenCalledTimes(1);
    expect(harness.focusNeighborThread).toHaveBeenCalledWith('thread-1');
    expect(harness.moveToFolder).toHaveBeenCalledWith(['thread-1'], 'f-projects', 'Projects');
  });

  it('offers topics only for mail from several accounts', () => {
    harness.pickerTarget = { itemType: 'thread', itemIds: ['thread-1', 'thread-2'] };
    harness.threads['thread-2'] = { accountId: 'other', mailboxNames: [] };

    const wrapper = render();

    expect(wrapper.findAll('.row').map((row) => row.text())).toEqual([topic.name]);
  });

  /**
   * App mounts the picker only once it has a target. The focus used to wait
   * for the target to change, which it never did, so `v` + typing went to the
   * list behind and Enter opened the thread instead of filing it.
   */
  it('takes the focus when it mounts already open', async () => {
    const wrapper = render(true);
    await flushPromises();

    expect(document.activeElement).toBe(wrapper.get('input').element);
    wrapper.unmount();
  });

  it('files into the filtered topic and closes on Enter', async () => {
    const wrapper = render();
    const input = wrapper.get('input');
    await input.setValue('Hunter');

    await input.trigger('keydown', { key: 'Enter' });

    expect(harness.closePicker).toHaveBeenCalledTimes(1);
    expect(harness.addToTopic).toHaveBeenCalledWith(['thread-1'], topic.id);
  });
});
