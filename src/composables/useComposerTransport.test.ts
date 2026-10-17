// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';

const backend = vi.hoisted(() => ({
  resolveSignature: vi.fn(),
  sendMessage: vi.fn(),
  supportsNativeCommands: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendResolveSignature: backend.resolveSignature,
  backendSendMessage: backend.sendMessage,
  backendCommandsAvailable: backend.supportsNativeCommands,
}));

import { useComposerTransport } from './useComposerTransport';
import type { BackendResolvedSignature } from '@/types/composer';

const request = {
  accountId: 'account-1',
  identityEmail: 'sender@example.com',
  threadId: 'thread-1',
  signatureId: 'signature-1',
  preference: 'auto' as const,
};

const resolved: BackendResolvedSignature = {
  signature_id: 'signature-1',
  variant: 'long' as const,
  body_text: 'Regards',
  body_html: '<p>Regards</p>',
  attachments: [],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.clearAllMocks();
  backend.supportsNativeCommands.mockReturnValue(true);
  backend.resolveSignature.mockResolvedValue(resolved);
  backend.sendMessage.mockResolvedValue({
    status: 'sent',
    thread_id: 'thread-1',
    message_id: 'message-1',
  });
});

describe('useComposerTransport', () => {
  it('reports the native capability and forwards a send payload', async () => {
    const transport = useComposerTransport();
    const payload = {
      accountId: 'account-1',
      to: ['recipient@example.com'],
      subject: 'Subject',
      body: 'Body',
      attachments: [],
    };

    expect(transport.commandsAvailable()).toBe(true);
    await expect(transport.sendMessage(payload)).resolves.toEqual({
      status: 'sent',
      thread_id: 'thread-1',
      message_id: 'message-1',
    });
    expect(backend.sendMessage).toHaveBeenCalledWith(payload);
  });

  it('keeps the newest signature response when requests finish out of order', async () => {
    const transport = useComposerTransport();
    const first = deferred<typeof resolved>();
    const second = deferred<typeof resolved>();
    backend.resolveSignature
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const firstRefresh = transport.refreshSignaturePreview(request);
    const secondRefresh = transport.refreshSignaturePreview({
      ...request,
      signatureId: 'signature-2',
      preference: 'short',
    });
    second.resolve({ ...resolved, signature_id: 'signature-2', variant: 'short' });
    await secondRefresh;
    first.resolve(resolved);
    await firstRefresh;

    expect(transport.resolvedSignature.value).toEqual({
      ...resolved,
      signature_id: 'signature-2',
      variant: 'short',
    });
    expect(transport.signaturePreviewLoading.value).toBe(false);
  });

  it('clears the preview without calling the backend for an incomplete request', async () => {
    const transport = useComposerTransport();
    transport.resolvedSignature.value = resolved;

    await transport.refreshSignaturePreview(null);

    expect(transport.resolvedSignature.value).toBeNull();
    expect(transport.signaturePreviewLoading.value).toBe(false);
    expect(backend.resolveSignature).not.toHaveBeenCalled();
  });

  it('clears a failed current preview and leaves stale failures harmless', async () => {
    const transport = useComposerTransport();
    const first = deferred<typeof resolved>();
    backend.resolveSignature.mockReturnValueOnce(first.promise);
    const firstRefresh = transport.refreshSignaturePreview(request);

    const secondRefresh = transport.refreshSignaturePreview(null);
    await secondRefresh;
    first.reject(new Error('preview failed'));
    await firstRefresh;

    expect(transport.resolvedSignature.value).toBeNull();
    expect(transport.signaturePreviewLoading.value).toBe(false);
  });
});
