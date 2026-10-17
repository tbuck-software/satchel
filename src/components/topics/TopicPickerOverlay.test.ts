// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  activeTopics: [] as Array<typeof topic>,
  addItemsToTopic: vi.fn(),
  closePicker: vi.fn(),
  create: vi.fn(),
}));

vi.mock('@/stores/topics', () => ({
  useTopicsStore: () => ({
    activeTopics: harness.activeTopics,
    pickerTarget: { itemType: 'message', itemIds: ['message-1'] },
    addItemsToTopic: harness.addItemsToTopic,
    byId: () => undefined,
    closePicker: harness.closePicker,
    create: harness.create,
    topicIdsFor: () => [],
  }),
}));

vi.mock('@/stores/ui', () => ({
  useUiStore: () => ({ registerOverlay: () => vi.fn() }),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({ addToTopic: vi.fn(), removeFromTopic: vi.fn() }),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ error: vi.fn(), success: vi.fn() }),
}));

import TopicPickerOverlay from './TopicPickerOverlay.vue';

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
  return mount(TopicPickerOverlay, {
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
