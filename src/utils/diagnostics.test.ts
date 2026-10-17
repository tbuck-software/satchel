// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { recordFrontendDiagnostic } from './diagnostics';
vi.mock('@/services/backend/misc', () => ({ backendRecordFrontendDiagnostic: vi.fn(async () => {}) }));
it('scans the DOM only for message lifecycle events', () => {
  const count = vi.spyOn(document, 'getElementsByTagName');
  const query = vi.spyOn(document, 'querySelectorAll');
  recordFrontendDiagnostic('threads.selection', 'j');
  recordFrontendDiagnostic('ui.click', 'row');
  expect(count).not.toHaveBeenCalled();
  expect(query).not.toHaveBeenCalled();
  recordFrontendDiagnostic('messages.loaded', 'thread');
  expect(count).toHaveBeenCalledTimes(2);
  expect(query).toHaveBeenCalledTimes(1);
  vi.restoreAllMocks();
});
