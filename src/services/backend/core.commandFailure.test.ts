import { afterEach, expect, it, vi } from 'vitest';
import { backendErrorKey } from '@/utils/backendErrors';

const transport = vi.hoisted(() => ({ sendCommand: vi.fn() }));
vi.mock('@/services/backend/transport', () => ({
  backendCommandsAvailable: () => true,
  sendCommand: transport.sendCommand,
}));

const { invoke, setCommandFailureListener } = await import('@/services/backend/core');

afterEach(() => setCommandFailureListener(null));

it('tells the failure listener which command failed and why, then rethrows', async () => {
  const listener = vi.fn();
  setCommandFailureListener(listener);
  transport.sendCommand.mockRejectedValueOnce({
    key: 'backend.errors.unexpected',
    params: { message: 'encode send payload' },
  });

  const thrown = await invoke('send_message').catch((error: unknown) => error);

  expect(listener).toHaveBeenCalledTimes(1);
  expect(listener.mock.calls[0]![0]).toBe('send_message');
  expect(listener.mock.calls[0]![1]).toBe(thrown);
  expect(backendErrorKey(thrown)).toBe('backend.errors.unexpected');
});

it('stays quiet when the command succeeds', async () => {
  const listener = vi.fn();
  setCommandFailureListener(listener);
  transport.sendCommand.mockResolvedValueOnce('ok');

  await expect(invoke('list_threads')).resolves.toBe('ok');
  expect(listener).not.toHaveBeenCalled();
});
