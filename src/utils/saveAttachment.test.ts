import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  save: vi.fn(),
  writeFile: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ success: harness.success, error: harness.error }),
}));
vi.mock('@/i18n/runtime', () => ({ t: (key: string) => key }));
vi.mock('@/utils/showInFolderAction', () => ({ showInFolderAction: () => undefined }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: harness.save }));
vi.mock('@tauri-apps/plugin-fs', () => ({ writeFile: harness.writeFile }));

import { saveAttachmentWithDialog } from './saveAttachment';

beforeEach(() => {
  vi.clearAllMocks();
  harness.save.mockResolvedValue(null);
});

describe('saveAttachmentWithDialog', () => {
  it('offers only a basename, never a folder taken from the mail', async () => {
    await saveAttachmentWithDialog('/Users/x/Library/LaunchAgents/a.plist', new Uint8Array([1]));

    expect(harness.save).toHaveBeenCalledWith({ defaultPath: 'a.plist' });
  });

  it('falls back when nothing usable is left', async () => {
    await saveAttachmentWithDialog('../', new Uint8Array([1]));

    expect(harness.save).toHaveBeenCalledWith({ defaultPath: 'attachment' });
  });
});
