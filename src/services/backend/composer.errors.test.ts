import { beforeEach, describe, expect, it, vi } from 'vitest';
import { backendListDrafts } from './composer';
import { sendCommand } from './transport';
import { backendErrorKey, backendErrorStringParam, translateBackendError } from '@/utils/backendErrors';

vi.mock('./transport', () => ({
  backendCommandsAvailable: () => true,
  sendCommand: vi.fn(),
}));

describe('draft recovery transport', () => {
  beforeEach(() => vi.resetAllMocks());

  it.each(['imapUidValidityMissing', 'imapUidValidityChanged'])(
    'preserves %s with its folder in both per-item data and command errors', async (key) => {
      const recovery = { key: `backend.errors.${key}`, params: { folder: 'Work/Drafts' } };
      const unavailable = { id: 'legacy', body: '', content_error: JSON.stringify(recovery) };
      vi.mocked(sendCommand).mockResolvedValueOnce([unavailable]);
      const drafts = await backendListDrafts();
      expect(sendCommand).toHaveBeenCalledWith('list_drafts', undefined);
      expect(translateBackendError(drafts[0]!.content_error)).toContain('Work/Drafts');
      expect(translateBackendError(drafts[0]!.content_error)).toMatch(/sync.*folder/i);

      vi.mocked(sendCommand).mockRejectedValueOnce(recovery);
      const error = await backendListDrafts().catch((error: unknown) => error);
      expect(backendErrorKey(error)).toBe(recovery.key);
      expect(backendErrorStringParam(error, 'folder')).toBe('Work/Drafts');
      expect((error as Error).message).toContain('Work/Drafts');
    },
  );
});
