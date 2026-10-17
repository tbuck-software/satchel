import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Attachment, Message } from '@/types/mail';

const harness = vi.hoisted(() => ({
  download: vi.fn(),
  openDefault: vi.fn(),
  openMessage: vi.fn(),
  quickLook: vi.fn(),
  save: vi.fn(),
  chooseDirectory: vi.fn(),
  writeFile: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendDownloadAttachment: harness.download,
  backendOpenAttachmentDefault: harness.openDefault,
  backendOpenMessageFileInWindow: harness.openMessage,
  backendToggleAttachmentQuickLook: harness.quickLook,
}));
vi.mock('@/utils/saveAttachment', () => ({
  saveAttachmentWithDialog: harness.save,
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: harness.chooseDirectory,
}));
vi.mock('@tauri-apps/plugin-fs', () => ({
  writeFile: harness.writeFile,
}));

import { useAttachmentActions } from './useAttachmentActions';

function attachment(overrides: Partial<Attachment> = {}): Attachment {
  return {
    id: 'attachment-1',
    filename: 'report.pdf',
    contentType: 'application/pdf',
    size: 12,
    ...overrides,
  };
}

function message(attachments: Attachment[]): Message {
  return {
    id: 'message-1',
    threadId: 'thread-1',
    from: { name: 'Sender', email: 'sender@example.com' },
    to: [],
    subject: 'Subject',
    bodyText: '',
    timestamp: '2026-08-30T00:00:00Z',
    isRead: true,
    attachments,
  };
}

function downloaded(filename: string, dataBase64 = 'AQI=') {
  return {
    filename,
    mime_type: 'application/octet-stream',
    data_base64: dataBase64,
    cached_path: null,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.download.mockResolvedValue(downloaded('report.pdf'));
  harness.openDefault.mockResolvedValue(undefined);
  harness.openMessage.mockResolvedValue(undefined);
  harness.quickLook.mockResolvedValue(false);
  harness.save.mockResolvedValue(undefined);
  harness.chooseDirectory.mockResolvedValue('/tmp/attachments');
  harness.writeFile.mockResolvedValue(undefined);
});

describe('useAttachmentActions', () => {
  it('downloads before saving and uses the attachment name as fallback', async () => {
    const actions = useAttachmentActions();
    harness.download.mockResolvedValue(downloaded(''));

    await actions.downloadAndSaveAttachment('attachment-1', 'fallback.bin');

    expect(harness.download).toHaveBeenCalledWith('attachment-1');
    // Bytes, not base64: above 4 MiB the payload arrives as a cache path, so
    // the save path decodes once here rather than at each call site.
    expect(harness.save).toHaveBeenCalledWith('fallback.bin', new Uint8Array([1, 2]));
  });

  it('signals the Quick Look fallback and detects cached message files', async () => {
    const actions = useAttachmentActions();
    harness.quickLook.mockResolvedValueOnce(false);

    await expect(actions.toggleQuickLook('attachment-1')).resolves.toBe(false);
    expect(actions.nestedMessagePath({
      ...downloaded('forwarded.eml'),
      mime_type: 'message/rfc822',
      cached_path: '/tmp/forwarded.eml',
    }, null, null)).toBe('/tmp/forwarded.eml');
    expect(actions.nestedMessagePath(downloaded('image.png'), null, 'image/png')).toBeNull();
  });

  it('filters body machinery from bulk downloads', () => {
    const actions = useAttachmentActions();
    const files = [
      attachment({ id: 'file', filename: 'report.pdf' }),
      attachment({ id: 'inline', filename: 'logo.png', isInline: true }),
      attachment({ id: 'calendar', filename: 'invite.ics', contentType: 'text/calendar' }),
      attachment({ id: 'signature', filename: 'smime.p7s', contentType: 'application/pkcs7-signature' }),
    ];

    expect(actions.listDownloadableAttachments([message(files)]).map((file) => file.id)).toEqual(['file']);
  });

  it('keeps duplicate names unique and continues after a failed attachment', async () => {
    const actions = useAttachmentActions();
    const files = [
      attachment({ id: 'first', filename: 'first.pdf' }),
      attachment({ id: 'failed', filename: 'failed.pdf' }),
      attachment({ id: 'second', filename: 'second.pdf' }),
    ];
    harness.download
      .mockResolvedValueOnce(downloaded('same.pdf'))
      .mockRejectedValueOnce(new Error('unreadable'))
      .mockResolvedValueOnce(downloaded('same.pdf'));

    await expect(actions.saveAttachmentsToDirectory(files, {
      fallbackFilename: 'attachment',
    })).resolves.toEqual({
      directory: '/tmp/attachments',
      saved: 2,
      failed: [files[1]],
    });
    expect(harness.writeFile).toHaveBeenNthCalledWith(
      1,
      '/tmp/attachments/same.pdf',
      expect.any(Uint8Array),
    );
    expect(harness.writeFile).toHaveBeenNthCalledWith(
      2,
      '/tmp/attachments/same (2).pdf',
      expect.any(Uint8Array),
    );
  });
});
