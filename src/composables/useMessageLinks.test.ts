import { beforeEach, describe, expect, it, vi } from 'vitest';

const push = vi.fn();
const toast = { success: vi.fn(), warning: vi.fn(), error: vi.fn() };
const backend = {
  backendGetThread: vi.fn(),
  backendMessageLinkTarget: vi.fn(),
  backendResolveMessageLink: vi.fn(),
};

vi.mock('vue-router', () => ({ useRouter: () => ({ push }) }));
vi.mock('@/composables/useToast', () => ({ useToast: () => toast }));
vi.mock('@/services/backend', () => backend);

const { useMessageLinks } = await import('./useMessageLinks');

describe('useMessageLinks', () => {
  const writeText = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  });

  it('opens the thread that holds the Message-ID', async () => {
    backend.backendResolveMessageLink.mockResolvedValue('thr-1');

    await useMessageLinks().open({ kind: 'message', messageId: 'abc@host' });

    expect(backend.backendResolveMessageLink).toHaveBeenCalledWith('abc@host');
    expect(push).toHaveBeenCalledWith({ name: 'thread', params: { id: 'thr-1' } });
  });

  it('says so instead of opening an empty thread view', async () => {
    backend.backendResolveMessageLink.mockResolvedValue(null);
    backend.backendGetThread.mockRejectedValue(new Error('thread not found'));

    await useMessageLinks().open({ kind: 'message', messageId: 'gone@host' });
    await useMessageLinks().open({ kind: 'thread', threadId: 'gone' });

    expect(push).not.toHaveBeenCalled();
    expect(toast.warning).toHaveBeenCalledTimes(2);
  });

  it('copies a Markdown link by Message-ID, by thread id only without one', async () => {
    backend.backendMessageLinkTarget
      .mockResolvedValueOnce({ thread_id: 'thr-1', subject: 'Invoice', message_id: 'abc@host' })
      .mockResolvedValueOnce({ thread_id: 'thr-2', subject: 'Local', message_id: null });

    await useMessageLinks().copy('thr-1');
    await useMessageLinks().copy('thr-2');

    expect(writeText.mock.calls).toEqual([
      ['[Invoice](satchel://message/abc@host)'],
      ['[Local](satchel://thread/thr-2)'],
    ]);
    expect(toast.success).toHaveBeenCalledTimes(2);
  });
});
