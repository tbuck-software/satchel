// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { backendUpsertSweepRule, backendDeleteSweepRule, type BackendSweepRule } from '@/services/backend';
import { answerConfirms, type DialogStub } from '@/test/dialogs';
import type { ConfirmRequest } from '@/composables/useConfirm';
import { useSenderSweep } from '@/composables/useSenderSweep';
import { useMailboxesStore } from '@/stores/mailboxes';
import SenderSweepDialog from './SenderSweepDialog.vue';

const harness = vi.hoisted(() => ({
  sweepSender: vi.fn(async () => 3),
  targets: 4,
  skippedShared: 0,
  skippedStarred: 0,
  rules: [] as BackendSweepRule[],
}));

vi.mock('vue-router', () => ({ useRoute: () => undefined, useRouter: () => undefined }));

vi.mock('@/composables/useThreadActions', () => ({
  useThreadActions: () => ({ sweepSender: harness.sweepSender }),
}));

vi.mock('@/services/backend', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/services/backend')>();
  return {
    ...actual,
    backendCommandsAvailable: () => true,
    backendSenderSweepSummary: vi.fn(async (input: { keep_latest: boolean; older_than_days: number | null }) => ({
      count: Math.max(0, harness.targets - (input.keep_latest ? 1 : 0) - (input.older_than_days ? 1 : 0)),
      skipped_shared: harness.skippedShared,
      skipped_starred: harness.skippedStarred,
    })),
    backendListSweepRules: vi.fn(async () => harness.rules),
    backendUpsertSweepRule: vi.fn(async (input: Record<string, unknown>) => ({
      id: 'sweep-1', last_run_at: null, last_swept_count: 0, created_at: '', updated_at: '',
      target_folder_name: input.target_folder_id ? 'Newsletters' : null, ...input,
    })),
    backendDeleteSweepRule: vi.fn(async () => {}),
  };
});

function rule(overrides: Partial<BackendSweepRule> = {}): BackendSweepRule {
  return {
    id: 'sweep-1', account_id: 'acc-1', sender_email: 'news@shop.example', action: 'archive',
    older_than_days: 30, keep_latest: false, target_folder_id: null, target_folder_name: null,
    last_run_at: null, last_swept_count: 0, created_at: '', updated_at: '', ...overrides,
  };
}

const mounted: Array<ReturnType<typeof mount>> = [];

async function open(request: Parameters<ReturnType<typeof useSenderSweep>['open']>[0]) {
  useSenderSweep().open(request);
  const wrapper = mount(SenderSweepDialog, { attachTo: document.body });
  mounted.push(wrapper);
  await flushPromises();
  return wrapper;
}

function dialog() {
  return document.body.querySelector<HTMLElement>('.sweep')!;
}

function button(text: string): HTMLButtonElement {
  const found = [...document.body.querySelectorAll<HTMLButtonElement>('button')]
    .find((candidate) => candidate.textContent?.includes(text));
  if (!found) throw new Error(`no button ${text}`);
  return found;
}

describe('sender sweep dialog', () => {
  let confirms: DialogStub<ConfirmRequest, boolean>;

  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    harness.targets = 4;
    harness.skippedShared = 0;
    harness.skippedStarred = 0;
    harness.rules = [];
    useSenderSweep().close();
    confirms = answerConfirms(true);
  });

  afterEach(() => {
    confirms.stop();
    for (const wrapper of mounted.splice(0)) wrapper.unmount();
    document.body.innerHTML = '';
  });

  it('counts the Inbox and runs a one-shot sweep from its number key', async () => {
    await open({ accountId: 'acc-1', email: 'News@Shop.Example', name: 'Shop News' });
    expect(dialog().textContent).toContain('Clean up mail from Shop News');
    // Whom it is about, always spelled out.
    expect(dialog().textContent).toContain('From news@shop.example');
    expect(dialog().textContent).toContain('4 conversations in the Inbox');

    dialog().dispatchEvent(new KeyboardEvent('keydown', { key: '3', bubbles: true, cancelable: true }));
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenCalledWith({
      accountId: 'acc-1', email: 'news@shop.example', action: 'archive', keepLatest: true,
    });
    // The receipt is the toast over the list; the dialog has left.
    expect(useSenderSweep().state.request).toBeNull();
  });

  it('says which of the sender\'s conversations it leaves alone', async () => {
    harness.skippedShared = 3;
    harness.skippedStarred = 1;
    await open({ accountId: 'acc-1', email: 'news@shop.example' });
    expect(dialog().textContent).toContain('3 conversations with other participants, your replies or drafts are left alone.');
    expect(dialog().textContent).toContain('One pinned conversation stays.');
  });

  it('offers moving all to Trash as a click', async () => {
    await open({ accountId: 'acc-1', email: 'news@shop.example' });
    button('Move all to Trash').click();
    await flushPromises();
    expect(harness.sweepSender).toHaveBeenCalledWith(expect.objectContaining({ action: 'trash', keepLatest: false }));
  });

  it('saves an age rule and shows how much it takes right away', async () => {
    await open({ accountId: 'acc-1', email: 'news@shop.example', focus: 'rule' });
    button('30 days').click();
    await flushPromises();
    expect(dialog().textContent).toContain('3 conversations qualify now.');
    button('Save rule').click();
    await flushPromises();
    expect(backendUpsertSweepRule).toHaveBeenCalledWith({
      account_id: 'acc-1', sender_email: 'news@shop.example', action: 'archive',
      older_than_days: 30, keep_latest: false, target_folder_id: null,
    });
    expect(useSenderSweep().state.request).toBeNull();
  });

  it('counts again right before asking about a trash rule', async () => {
    await open({ accountId: 'acc-1', email: 'news@shop.example' });
    button('Move to Trash').click();
    await flushPromises();
    // More mail arrived since the preview.
    harness.targets = 9;
    button('Save rule').click();
    await flushPromises();
    expect(confirms.asked).toHaveBeenCalledWith(expect.objectContaining({
      title: 'Save the rule and move 8 conversations to Trash now?',
    }));
  });

  it('asks with the count before a trash rule runs, and saves nothing on No', async () => {
    await open({ accountId: 'acc-1', email: 'news@shop.example' });
    button('Move to Trash').click();
    await flushPromises();
    confirms.reply(false);
    button('Save rule').click();
    await flushPromises();
    expect(confirms.asked).toHaveBeenCalledWith(expect.objectContaining({
      title: 'Save the rule and move 3 conversations to Trash now?',
      tone: 'danger',
    }));
    expect(backendUpsertSweepRule).not.toHaveBeenCalled();
    confirms.reply(true);
    button('Save rule').click();
    await flushPromises();
    expect(backendUpsertSweepRule).toHaveBeenCalledWith(expect.objectContaining({ action: 'trash', older_than_days: 10 }));
  });

  it('asks a move rule that lost its folder for a new one', async () => {
    harness.rules = [rule({ action: 'move', older_than_days: null, target_folder_id: null })];
    await open({ accountId: 'acc-1', email: 'news@shop.example' });
    await flushPromises();
    expect(dialog().textContent).toContain('Its folder is gone.');
    expect(button('Update rule').disabled).toBe(true);
  });

  it('needs a folder before a move rule can be saved', async () => {
    const mailboxes = useMailboxesStore();
    mailboxes.mailboxes = [
      { id: 'f-inbox', account_id: 'acc-1', account_email: 'me@example.com', name: 'INBOX', mailbox_type: 'inbox', thread_count: 0, unread_count: 0 },
      { id: 'f-news', account_id: 'acc-1', account_email: 'me@example.com', name: 'Newsletters', mailbox_type: 'custom', thread_count: 0, unread_count: 0 },
      { id: 'f-other', account_id: 'acc-2', account_email: 'you@example.com', name: 'Elsewhere', mailbox_type: 'custom', thread_count: 0, unread_count: 0 },
    ] as typeof mailboxes.mailboxes;
    await open({ accountId: 'acc-1', email: 'news@shop.example' });
    button('Move to folder').click();
    await flushPromises();
    expect(button('Save rule').disabled).toBe(true);
    // Only this account's folders, and never the Inbox itself.
    expect(dialog().textContent).not.toContain('Elsewhere');
    button('Newsletters').click();
    await flushPromises();
    button('Save rule').click();
    await flushPromises();
    expect(backendUpsertSweepRule).toHaveBeenCalledWith(expect.objectContaining({
      action: 'move', older_than_days: null, target_folder_id: 'f-news',
    }));
  });

  it('edits and removes an existing rule', async () => {
    harness.rules = [rule({ action: 'trash', older_than_days: 10, keep_latest: true })];
    await open({ accountId: 'acc-1', email: 'news@shop.example' });
    await flushPromises();
    expect(button('Update rule')).toBeTruthy();
    button('Remove rule').click();
    await flushPromises();
    expect(backendDeleteSweepRule).toHaveBeenCalledWith('sweep-1');
  });
});
