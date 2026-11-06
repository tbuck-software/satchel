// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendCleanupSender } from '@/services/backend';
import { useSenderSweep } from '@/composables/useSenderSweep';
import { useUiStore } from '@/stores/ui';
import { answerConfirms, type DialogStub } from '@/test/dialogs';
import type { ConfirmRequest } from '@/composables/useConfirm';
import CleanupView from './CleanupView.vue';

const harness = vi.hoisted(() => ({
  push: vi.fn(),
  replace: vi.fn(),
  query: {} as Record<string, string>,
  sweepSender: vi.fn(async () => 1),
  unsubscribe: vi.fn(async () => ({ method: 'http-one-click', target: 'https://shop.example/u' })),
  senders: [] as BackendCleanupSender[],
  listInputs: [] as unknown[],
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ push: harness.push, replace: harness.replace }),
  useRoute: () => ({ query: harness.query }),
}));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({ sweepSender: harness.sweepSender }),
}));

vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({ unsubscribe: harness.unsubscribe }),
}));

vi.mock('@/services/backend', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/backend')>();
  return {
    ...actual,
    backendCommandsAvailable: () => true,
    backendListCleanupSenders: vi.fn(async (input: unknown) => {
      harness.listInputs.push(input);
      return harness.senders;
    }),
    backendListSweepRules: vi.fn(async () => []),
    backendSenderSweepSummary: vi.fn(async () => ({ count: 38, skipped_shared: 2, skipped_starred: 0 })),
    backendListAccounts: vi.fn(async () => []),
  };
});

function sender(overrides: Partial<BackendCleanupSender>): BackendCleanupSender {
  return {
    account_id: 'acc-1',
    sender_email: 'news@shop.example',
    sender_name: 'Shop News',
    message_count: 40,
    unread_count: 30,
    thread_count: 38,
    newest_date: '2026-11-04T10:00:00+00:00',
    unsubscribe_message_id: 'm-1',
    rule_id: null,
    ...overrides,
  };
}

function press(key: string) {
  window.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
}

const mounted: Array<ReturnType<typeof mount>> = [];

async function mountView() {
  const wrapper = mount(CleanupView, { attachTo: document.body });
  mounted.push(wrapper);
  await flushPromises();
  return wrapper;
}

describe('cleanup view', () => {
  let confirms: DialogStub<ConfirmRequest, boolean>;

  beforeEach(() => {
    confirms = answerConfirms(true);
    setActivePinia(createPinia());
    vi.clearAllMocks();
    harness.query = {};
    harness.listInputs.length = 0;
    harness.senders = [
      sender({}),
      sender({ sender_email: 'alerts@bank.example', sender_name: null, message_count: 12, unread_count: 0, thread_count: 1, unsubscribe_message_id: null }),
    ];
    useSenderSweep().close();
  });

  afterEach(() => {
    confirms.stop();
    // A rebound key is persisted; the next test starts from the defaults.
    localStorage.clear();
    for (const wrapper of mounted.splice(0)) wrapper.unmount();
    document.body.innerHTML = '';
  });

  it('lists the busiest senders of the last 90 days with what can be done about them', async () => {
    const wrapper = await mountView();
    expect(harness.listInputs[0]).toMatchObject({ days: 90, account_id: null });
    const rows = wrapper.findAll('.cleanup-row');
    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain('Shop News');
    expect(rows[0]!.text()).toContain('40 messages');
    expect(rows[0]!.text()).toContain('75% unread');
    expect(rows[0]!.text()).toContain('Unsubscribe available');
    expect(rows[1]!.text()).toContain('alerts@bank.example');
    expect(rows[1]!.text()).not.toContain('Unsubscribe available');
  });

  it('scopes to one account from the sidebar menu', async () => {
    harness.query = { account: 'acc-2' };
    await mountView();
    expect(harness.listInputs[0]).toMatchObject({ account_id: 'acc-2' });
  });

  it('moves with j/k and acts on the focused sender with the action keys', async () => {
    const wrapper = await mountView();

    press('j');
    press('e');
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenLastCalledWith({
      accountId: 'acc-1', email: 'alerts@bank.example', action: 'archive', keepLatest: false,
    });

    press('k');
    press('#');
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenLastCalledWith({
      accountId: 'acc-1', email: 'news@shop.example', action: 'trash', keepLatest: false,
    });

    press('p');
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenLastCalledWith({
      accountId: 'acc-1', email: 'news@shop.example', action: 'archive', keepLatest: true,
    });

    press('U');
    await flushPromises();
    expect(harness.unsubscribe).toHaveBeenCalledWith('m-1');
    expect(wrapper.text()).toContain('Unsubscribed');

    press('R');
    expect(useSenderSweep().state.request).toMatchObject({ email: 'news@shop.example', focus: 'rule' });
  });

  it('leaves the keys alone while a dialog covers the page', async () => {
    await mountView();
    const ui = useUiStore();
    const unregister = ui.registerOverlay({ isOpen: () => true, dismiss: () => {} });
    press('e');
    await flushPromises();
    expect(harness.sweepSender).not.toHaveBeenCalled();
    unregister();
  });

  it('runs the row buttons through the same sweep', async () => {
    const wrapper = await mountView();
    const buttons = wrapper.findAll('.cleanup-row')[0]!.findAll('.cleanup-row__actions button');
    const byText = (text: string) => buttons.find((button) => button.text().startsWith(text))!;
    await byText('Archive all').trigger('click');
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenLastCalledWith(expect.objectContaining({ action: 'archive', keepLatest: false }));
    await byText('Move all to Trash').trigger('click');
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenLastCalledWith(expect.objectContaining({ action: 'trash' }));
    await byText('Keep latest').trigger('click');
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenLastCalledWith(expect.objectContaining({ keepLatest: true }));
    await byText('From now on').trigger('click');
    expect(useSenderSweep().state.request).toMatchObject({ focus: 'rule' });
  });

  it('asks with the count before moving a sender\'s mail to Trash', async () => {
    await mountView();
    confirms.reply(false);
    press('#');
    await flushPromises();
    expect(confirms.asked).toHaveBeenCalledWith(expect.objectContaining({
      title: 'Move 38 conversations from Shop News to Trash?',
      tone: 'danger',
    }));
    expect(harness.sweepSender).not.toHaveBeenCalled();
  });

  it('reads its keys from the bindings, so a rebound key moves with them', async () => {
    const { useSettingsStore } = await import('@/stores/settings');
    const settings = useSettingsStore();
    settings.keybindings = { ...settings.keybindings, cleanupKeepLatest: 'q' };
    const wrapper = await mountView();
    expect(wrapper.find('.cleanup-legend').text()).toContain('q');
    press('p');
    await flushPromises();
    expect(harness.sweepSender).not.toHaveBeenCalled();
    press('q');
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenLastCalledWith(expect.objectContaining({ keepLatest: true }));
  });

  it('shows the empty state when nobody filled the Inbox', async () => {
    harness.senders = [];
    const wrapper = await mountView();
    expect(wrapper.text()).toContain('Nothing to clean up');
  });
});
