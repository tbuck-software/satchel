// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

describe('mock local-only workflow accounts', () => {
  beforeEach(() => { vi.resetModules(); window.history.replaceState({}, '', '/'); });
  afterEach(() => { window.history.replaceState({}, '', '/'); });

  it('lists none unless the preview asks for one', async () => {
    const { sendCommand } = await import('./mock');
    await expect(sendCommand<string[]>('list_local_only_workflow_accounts')).resolves.toEqual([]);
  });

  it('lists the work account with its reason for ?mockWorkflowSync=…', async () => {
    for (const [param, reason] of [['local', 'no_custom_keywords'], ['store_refused', 'store_refused'], ['gmail_imap', 'gmail_imap']]) {
      vi.resetModules();
      window.history.replaceState({}, '', `/?mockWorkflowSync=${param}`);
      const { sendCommand } = await import('./mock');
      await expect(sendCommand('list_local_only_workflow_accounts')).resolves.toEqual([{ account_id: 'acc-work', reason }]);
    }
  });
});
