// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { backendRecordFrontendDiagnostic } from '@/services/backend/misc';
import { normalizeInvokeError, setCommandFailureListener } from '@/services/backend/core';
import { installCommandFailureDiagnostics } from './commandFailureDiagnostics';
vi.mock('@/services/backend/misc', () => ({ backendRecordFrontendDiagnostic: vi.fn(async () => {}) }));
vi.mock('@/services/backend/core', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend/core')>(),
  setCommandFailureListener: vi.fn(),
}));

/** A click and the failure behind it belong on neighbouring lines of the same log. */
it('logs a failed command next to the clicks, once per burst and never about itself', () => {
  vi.useFakeTimers();
  installCommandFailureDiagnostics();
  const listener = vi.mocked(setCommandFailureListener).mock.calls[0]![0]!;
  const record = vi.mocked(backendRecordFrontendDiagnostic);
  record.mockClear();
  const failure = normalizeInvokeError({
    key: 'backend.errors.unexpected',
    params: { message: 'encode send payload' },
  });

  listener('send_message', failure);
  listener('send_message', failure);
  listener('record_frontend_diagnostic', failure);

  expect(record).toHaveBeenCalledTimes(1);
  expect(record.mock.calls[0]![0]).toMatchObject({
    event: 'command.failed',
    detail: 'command=send_message key=backend.errors.unexpected message=encode send payload',
    level: 'warn',
  });

  vi.advanceTimersByTime(10_000);
  listener('send_message', failure);
  expect(record).toHaveBeenCalledTimes(2);
  vi.useRealTimers();
});
