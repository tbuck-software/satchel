// @vitest-environment jsdom

import { nextTick, ref } from 'vue';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendOutgoingAttachment } from '@/types/composer';
import {
  useComposerInlineAttachments,
  type ComposerInlineAttachmentEditor,
} from './useComposerInlineAttachments';

const image = {
  filename: 'photo.png',
  mimeType: 'image/png',
  contentBase64: 'aW1hZ2U=',
  dataUrl: 'data:image/png;base64,aW1hZ2U=',
};

function attachment(overrides: Partial<BackendOutgoingAttachment> = {}): BackendOutgoingAttachment {
  return {
    filename: 'photo.png',
    mime_type: 'image/png',
    content_base64: image.contentBase64,
    content_id: 'photo@satchel',
    inline: true,
    ...overrides,
  };
}

function createHarness(initial: BackendOutgoingAttachment[] = []) {
  const attachments = ref(initial);
  const editor: ComposerInlineAttachmentEditor = {
    insertAttachmentImage: vi.fn(),
    removeAttachmentImage: vi.fn(),
  };
  const inline = useComposerInlineAttachments(attachments, () => editor);
  return { attachments, editor, inline };
}

describe('useComposerInlineAttachments', () => {
  beforeEach(() => {
    vi.stubGlobal('crypto', { randomUUID: vi.fn(() => 'generated-id') });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('hydrates and serializes only inline CID attachments', () => {
    const { inline } = createHarness([
      attachment(),
      attachment({
        filename: 'report.pdf',
        mime_type: 'application/pdf',
        content_id: 'file@satchel',
        inline: false,
      }),
    ]);
    const dataUrl = 'data:image/png;base64,aW1hZ2U=';

    expect(inline.hydrateInlineImages('<img src="cid:photo@satchel"><a href="cid:file@satchel">file</a>', [
      attachment(),
      attachment({
        filename: 'report.pdf',
        mime_type: 'application/pdf',
        content_id: 'file@satchel',
        inline: false,
      }),
    ])).toBe(`<img src="${dataUrl}"><a href="cid:file@satchel">file</a>`);
    expect(inline.serializeInlineImages(`<img src="${dataUrl}"><a href="cid:file@satchel">file</a>`))
      .toBe('<img src="cid:photo@satchel"><a href="cid:file@satchel">file</a>');
  });

  it('adds inline images, preserves an existing CID, and replaces an existing payload in place', () => {
    const regularAttachment = attachment({
      filename: 'document.pdf',
      content_base64: 'cGRm',
      content_id: null,
      inline: false,
    });
    const existing = attachment({ filename: 'old.png', content_id: 'existing@satchel' });
    const { attachments, inline } = createHarness([
      regularAttachment,
      existing,
    ]);

    inline.onInlineImageAdded({ ...image, filename: 'updated.png' });

    expect(attachments.value).toEqual([
      regularAttachment,
      {
        filename: 'updated.png',
        mime_type: 'image/png',
        content_base64: image.contentBase64,
        content_id: 'existing@satchel',
        inline: true,
      },
    ]);

    inline.onInlineImageAdded({ ...image, contentBase64: 'new-image', filename: 'new.png' });
    expect(attachments.value.at(-1)).toEqual({
      filename: 'new.png',
      mime_type: 'image/png',
      content_base64: 'new-image',
      content_id: 'satchel-generated-id@inline',
      inline: true,
    });
  });

  it('moves an image to a regular attachment and clears its CID', () => {
    const { attachments, inline } = createHarness([attachment()]);

    inline.onImageToAttachment({ ...image, filename: 'download.png' });

    expect(attachments.value).toEqual([{
      filename: 'download.png',
      mime_type: 'image/png',
      content_base64: image.contentBase64,
      content_id: null,
      inline: false,
    }]);
  });

  it('defers editor insertion and removal until the next tick', async () => {
    const { editor, inline } = createHarness();
    const inlineAttachment = attachment();

    inline.onAttachmentInsertedInline(inlineAttachment);
    inline.onInlineAttachmentRemoved(inlineAttachment);
    expect(editor.insertAttachmentImage).not.toHaveBeenCalled();
    expect(editor.removeAttachmentImage).not.toHaveBeenCalled();

    await nextTick();
    expect(editor.insertAttachmentImage).toHaveBeenCalledWith(inlineAttachment);
    expect(editor.removeAttachmentImage).toHaveBeenCalledWith(inlineAttachment);
  });

  it('reads pasted files and appends base64 regular attachments', async () => {
    const existing = attachment({ filename: 'existing.png' });
    const { attachments, inline } = createHarness([existing]);
    const file = {
      name: 'notes.bin',
      type: 'application/octet-stream',
      arrayBuffer: vi.fn(async () => new Uint8Array([0, 1, 255]).buffer),
    } as unknown as File;
    const untitled = {
      name: 'untitled',
      type: '',
      arrayBuffer: vi.fn(async () => new Uint8Array([65]).buffer),
    } as unknown as File;

    await inline.addPastedFiles([file, untitled]);

    expect(attachments.value).toEqual([
      existing,
      {
        filename: 'notes.bin',
        mime_type: 'application/octet-stream',
        content_base64: 'AAH/',
        inline: false,
      },
      {
        filename: 'untitled',
        mime_type: 'application/octet-stream',
        content_base64: 'QQ==',
        inline: false,
      },
    ]);
  });

  it('preserves attachment changes made while pasted files are being read', async () => {
    let finishRead: ((value: ArrayBuffer) => void) | undefined;
    const pendingBuffer = new Promise<ArrayBuffer>((resolve) => {
      finishRead = resolve;
    });
    const existing = attachment({ filename: 'existing.png' });
    const concurrent = attachment({ filename: 'concurrent.png', content_base64: 'bmV3' });
    const { attachments, inline } = createHarness([existing]);
    const file = {
      name: 'pasted.txt',
      type: 'text/plain',
      arrayBuffer: vi.fn(() => pendingBuffer),
    } as unknown as File;

    const adding = inline.addPastedFiles([file]);
    attachments.value = [...attachments.value, concurrent];
    finishRead?.(new Uint8Array([65]).buffer);
    await adding;

    expect(attachments.value).toEqual([
      existing,
      concurrent,
      {
        filename: 'pasted.txt',
        mime_type: 'text/plain',
        content_base64: 'QQ==',
        inline: false,
      },
    ]);
  });

  it('serializes concurrent paste operations in invocation order', async () => {
    let finishFirst: ((value: ArrayBuffer) => void) | undefined;
    const firstBuffer = new Promise<ArrayBuffer>((resolve) => {
      finishFirst = resolve;
    });
    const first = {
      name: 'first.txt',
      type: 'text/plain',
      arrayBuffer: vi.fn(() => firstBuffer),
    } as unknown as File;
    const second = {
      name: 'second.txt',
      type: 'text/plain',
      arrayBuffer: vi.fn(async () => new Uint8Array([66]).buffer),
    } as unknown as File;
    const { attachments, inline } = createHarness();

    const addingFirst = inline.addPastedFiles([first]);
    const addingSecond = inline.addPastedFiles([second]);
    await Promise.resolve();
    expect(second.arrayBuffer).not.toHaveBeenCalled();

    finishFirst?.(new Uint8Array([65]).buffer);
    await Promise.all([addingFirst, addingSecond]);

    expect(attachments.value.map((value) => value.filename)).toEqual([
      'first.txt',
      'second.txt',
    ]);
  });

  it('continues the paste queue after a failed file read', async () => {
    const failed = {
      name: 'failed.txt',
      type: 'text/plain',
      arrayBuffer: vi.fn(async () => { throw new Error('read failed'); }),
    } as unknown as File;
    const next = {
      name: 'next.txt',
      type: 'text/plain',
      arrayBuffer: vi.fn(async () => new Uint8Array([65]).buffer),
    } as unknown as File;
    const { attachments, inline } = createHarness();

    await expect(inline.addPastedFiles([failed])).rejects.toThrow('read failed');
    await inline.addPastedFiles([next]);

    expect(attachments.value.map((value) => value.filename)).toEqual(['next.txt']);
  });
});
