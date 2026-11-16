// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendThreadPage } from './threads';
import type { BackendCleanupSender, BackendSweepRule, BackendSweepSummary, BackendSweepTarget, BackendThreadSweepSender } from './sweep';

/**
 * The mock answers the sender sweep commands with the semantics of
 * `sender_sweep.rs`: Inbox threads only, newest first, keep-latest before the
 * age cutoff, one rule per sender and account that runs once when saved.
 */
describe('mock sender sweep', () => {
  beforeEach(() => {
    vi.resetModules();
    window.history.replaceState({}, '', '/?mockExtraThreads=0');
  });

  async function inboxIds(send: <T>(command: string, payload?: Record<string, unknown>) => Promise<T>) {
    const page = await send<BackendThreadPage>('list_threads', { input: {
      mailbox_kind: 'section', mailbox_value: 'inbox', limit: 1000,
    } });
    return page.threads.map((thread) => thread.id);
  }

  it('lists senders by volume and names what archive-all would take', async () => {
    const { sendCommand } = await import('./mock');
    const senders = await sendCommand<BackendCleanupSender[]>('list_cleanup_senders', { input: { days: 3650 } });
    expect(senders.length).toBeGreaterThan(0);
    for (let index = 1; index < senders.length; index += 1) {
      expect(senders[index - 1]!.message_count).toBeGreaterThanOrEqual(senders[index]!.message_count);
    }
    const inbox = new Set(await inboxIds(sendCommand));
    for (const sender of senders) {
      const targets = await sendCommand<BackendSweepTarget[]>('list_sender_sweep_targets', { input: {
        account_id: sender.account_id, sender_email: sender.sender_email,
      } });
      const summary = await sendCommand<BackendSweepSummary>('sender_sweep_summary', { input: {
        account_id: sender.account_id, sender_email: sender.sender_email,
      } });
      expect(summary.count).toBe(targets.length);
      expect(targets.every((target) => inbox.has(target.thread_id))).toBe(true);
      const kept = await sendCommand<BackendSweepTarget[]>('list_sender_sweep_targets', { input: {
        account_id: sender.account_id, sender_email: sender.sender_email, keep_latest: true,
      } });
      expect(kept.map((target) => target.thread_id)).toEqual(targets.slice(1).map((target) => target.thread_id));
    }
    // A newsletter carries its unsubscribe header in the fixture.
    expect(senders.some((sender) => sender.unsubscribe_message_id)).toBe(true);
    }, 20000);

  it('saves a rule per sender, runs it once and leaves other senders alone', async () => {
    const { sendCommand } = await import('./mock');
    const senders = await sendCommand<BackendCleanupSender[]>('list_cleanup_senders', { input: { days: 3650 } });
    const sender = senders.find((entry) => entry.thread_count > 0)!;
    const before = await inboxIds(sendCommand);
    const targets = await sendCommand<BackendSweepTarget[]>('list_sender_sweep_targets', { input: {
      account_id: sender.account_id, sender_email: sender.sender_email,
    } });

    const rule = await sendCommand<BackendSweepRule>('upsert_sweep_rule', { input: {
      account_id: sender.account_id, sender_email: sender.sender_email.toUpperCase(), action: 'archive',
    } });
    expect(rule.sender_email).toBe(sender.sender_email);
    expect(rule.last_swept_count).toBe(targets.length);

    const after = await inboxIds(sendCommand);
    const swept = new Set(targets.map((target) => target.thread_id));
    expect(after).toEqual(before.filter((id) => !swept.has(id)));

    // A second save replaces the rule instead of adding one.
    await sendCommand('upsert_sweep_rule', { input: {
      account_id: sender.account_id, sender_email: sender.sender_email, action: 'trash', older_than_days: 30,
    } });
    await expect(sendCommand('upsert_sweep_rule', { input: {
      account_id: sender.account_id, sender_email: sender.sender_email, action: 'trash',
    } })).rejects.toThrow('senderSweepTrashNeedsAge');
    const rules = await sendCommand<BackendSweepRule[]>('list_sweep_rules');
    expect(rules).toHaveLength(1);
    expect(rules[0]).toMatchObject({ action: 'trash', older_than_days: 30 });

    await sendCommand('delete_sweep_rule', { ruleId: rule.id });
    await expect(sendCommand('delete_sweep_rule', { ruleId: rule.id })).rejects.toThrow('senderSweepRuleNotFound');
  });

  it('refuses what the backend refuses, with its keys', async () => {
    const { sendCommand } = await import('./mock');
    await expect(sendCommand('upsert_sweep_rule', { input: {
      account_id: 'acc-work', sender_email: 'news@shop.example', action: 'explode',
    } })).rejects.toThrow('senderSweepActionInvalid');
    await expect(sendCommand('upsert_sweep_rule', { input: {
      account_id: 'acc-work', sender_email: 'nobody', action: 'archive',
    } })).rejects.toThrow('senderSweepSenderInvalid');
    const accounts = await sendCommand<Array<{ id: string; email: string }>>('list_accounts');
    const own = accounts.find((account) => account.id === 'acc-work')!;
    await expect(sendCommand('upsert_sweep_rule', { input: {
      account_id: own.id, sender_email: own.email, action: 'archive',
    } })).rejects.toThrow('senderSweepOwnAddress');
    await expect(sendCommand('sender_sweep_summary', { input: {
      account_id: own.id, sender_email: own.email,
    } })).rejects.toThrow('senderSweepOwnAddress');
    await expect(sendCommand('upsert_sweep_rule', { input: {
      account_id: 'acc-work', sender_email: 'news@shop.example', action: 'move', target_folder_id: 'other:Folder',
    } })).rejects.toThrow('senderSweepFolderInvalid');
  });

  it('names who started a thread, never the user', async () => {
    const { sendCommand } = await import('./mock');
    const accounts = await sendCommand<Array<{ id: string; email: string }>>('list_accounts');
    const page = await sendCommand<BackendThreadPage>('list_threads', { input: { limit: 50 } });
    for (const thread of page.threads) {
      const sender = await sendCommand<BackendThreadSweepSender | null>('thread_sweep_sender', { threadId: thread.id });
      if (!sender) continue;
      expect(sender.account_id).toBe(thread.account_id);
      expect(accounts.some((account) => account.email.toLowerCase() === sender.email)).toBe(false);
    }
  });

  it('unsubscribes through the newest message with the header', async () => {
    const { sendCommand } = await import('./mock');
    const senders = await sendCommand<BackendCleanupSender[]>('list_cleanup_senders', { input: { days: 3650 } });
    const newsletter = senders.find((sender) => sender.unsubscribe_message_id)!;
    await expect(sendCommand('unsubscribe', { input: { message_id: newsletter.unsubscribe_message_id } }))
      .resolves.toMatchObject({ method: 'http-one-click' });
  });
});
