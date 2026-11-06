// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

describe('mock local-only workflow accounts', () => {
  beforeEach(() => { vi.resetModules(); window.history.replaceState({}, '', '/'); });
  afterEach(() => { window.history.replaceState({}, '', '/'); });

  it('lists none unless the preview asks for one', async () => {
    const { sendCommand } = await import('./mock');
    await expect(sendCommand<string[]>('list_local_only_workflow_accounts')).resolves.toEqual([]);
  });

  it('lists the work account with ?mockWorkflowSync=local', async () => {
    window.history.replaceState({}, '', '/?mockWorkflowSync=local');
    const { sendCommand } = await import('./mock');
    await expect(sendCommand<string[]>('list_local_only_workflow_accounts')).resolves.toEqual(['acc-work']);
  });
});
