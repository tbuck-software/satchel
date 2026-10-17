import { beforeEach, describe, expect, it, vi } from 'vitest';

/*
 * SEC-18. The backend refuses a type the operating system runs until the user
 * has seen the whole file name and said yes. Before this the frontend never
 * sent the confirmation, so an executable attachment could not be opened at
 * all — and `Rechnung.pdf.exe` never got its question.
 */

const harness = vi.hoisted(() => ({
  openDefault: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendDownloadAttachment: vi.fn(),
  backendOpenAttachmentDefault: harness.openDefault,
  backendOpenMessageFileInWindow: vi.fn(),
  backendToggleAttachmentQuickLook: vi.fn(),
}));
vi.mock('@/utils/saveAttachment', () => ({ saveAttachmentWithDialog: vi.fn() }));
vi.mock('@/i18n/runtime', () => ({ t: (key: string) => key }));

import { useAttachmentActions } from './useAttachmentActions';
import { useConfirm } from './useConfirm';

/** The refusal the backend sends: a key plus the parameters for the question. */
function refusal(key: string, filename: string, extension: string) {
  const error = new Error(key) as Error & { cause?: unknown };
  error.cause = { key, params: { filename, extension } };
  return error;
}

/** Wait until the dialog is pending, then answer it. */
async function answer(confirmed: boolean) {
  const { state, answer: reply } = useConfirm();
  for (let turn = 0; turn < 8 && state.pending === null; turn += 1) {
    await Promise.resolve();
  }
  expect(state.pending).not.toBeNull();
  reply(confirmed);
}

describe('opening an executable attachment', () => {
  beforeEach(() => {
    harness.openDefault.mockReset();
    useConfirm().answer(false);
  });

  it('asks before handing a program to the operating system', async () => {
    harness.openDefault
      .mockRejectedValueOnce(refusal('backend.errors.attachmentExecutableNotConfirmed', 'setup.exe', 'exe'))
      .mockResolvedValueOnce(undefined);

    const opening = useAttachmentActions().openAttachmentDefault('attachment-1');
    await answer(true);
    await opening;

    expect(harness.openDefault).toHaveBeenCalledTimes(2);
    expect(harness.openDefault).toHaveBeenLastCalledWith('attachment-1', true);
  });

  it('does not open it when the answer is no', async () => {
    harness.openDefault.mockRejectedValueOnce(
      refusal('backend.errors.attachmentExecutableNotConfirmed', 'setup.exe', 'exe'),
    );

    const opening = useAttachmentActions().openAttachmentDefault('attachment-1');
    await answer(false);
    await expect(opening).resolves.toBeUndefined();

    // Once — the refused attempt. No second call means `open_path` never ran.
    expect(harness.openDefault).toHaveBeenCalledTimes(1);
  });

  it('names the full file name, which is where a second extension hides', async () => {
    harness.openDefault.mockRejectedValueOnce(
      refusal(
        'backend.errors.attachmentExecutableDisguisedNotConfirmed',
        'Rechnung.pdf.exe',
        'exe',
      ),
    );

    const opening = useAttachmentActions().openAttachmentDefault('attachment-1');
    const { state } = useConfirm();
    for (let turn = 0; turn < 8 && state.pending === null; turn += 1) {
      await Promise.resolve();
    }

    expect(state.pending?.detail).toBe('Rechnung.pdf.exe');
    expect(state.pending?.body).toBe('mail.attachment.executableDisguisedBody');
    expect(state.pending?.tone).toBe('danger');

    useConfirm().answer(false);
    await opening;
  });

  it('lets every other failure through untouched', async () => {
    const other = new Error('disk full');
    harness.openDefault.mockRejectedValueOnce(other);

    await expect(useAttachmentActions().openAttachmentDefault('attachment-1')).rejects.toBe(other);
    expect(useConfirm().state.pending).toBeNull();
  });
});
