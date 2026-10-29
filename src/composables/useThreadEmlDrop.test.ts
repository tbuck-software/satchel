import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ref } from 'vue';
import type { BackendOutgoingAttachment } from '@/types/composer';
import { MAX_TOTAL_ATTACHMENT_BYTES } from '@/utils/attachmentLimits';
import { THREAD_DRAG_TYPE } from '@/utils/threadDrag';

const harness = vi.hoisted(() => ({ fetch: vi.fn() }));

vi.mock('@/services/backend/composer', () => ({
  backendThreadEmlAttachments: harness.fetch,
}));

import { useThreadEmlDrop } from './useThreadEmlDrop';

function eml(filename: string, bytes = 3): BackendOutgoingAttachment {
  return {
    filename,
    mime_type: 'message/rfc822',
    content_base64: 'A'.repeat(Math.ceil((bytes * 4) / 3)),
    content_id: null,
    inline: false,
  };
}

function dragEvent(data: Record<string, string>) {
  return {
    dataTransfer: {
      types: Object.keys(data),
      getData: (type: string) => data[type] ?? '',
      dropEffect: 'none',
    },
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  } as unknown as DragEvent & { preventDefault: ReturnType<typeof vi.fn>; stopPropagation: ReturnType<typeof vi.fn> };
}

function setup(initial: BackendOutgoingAttachment[] = [], disabled = false) {
  const attachments = ref<BackendOutgoingAttachment[]>(initial);
  const feedback = { loading: vi.fn(), done: vi.fn(), error: vi.fn() };
  const drop = useThreadEmlDrop(attachments, feedback, () => disabled);
  return { attachments, feedback, drop };
}

async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe('useThreadEmlDrop', () => {
  beforeEach(() => {
    harness.fetch.mockReset();
  });

  it('attaches the original of every dropped thread', async () => {
    harness.fetch.mockResolvedValue([eml('Paket.eml'), eml('Rechnung.eml')]);
    const { attachments, feedback, drop } = setup([eml('existing.pdf')]);

    drop.onDrop(dragEvent({ [THREAD_DRAG_TYPE]: 'th-1,th-2' }));
    await settle();

    expect(harness.fetch).toHaveBeenCalledWith(['th-1', 'th-2']);
    expect(attachments.value.map((item) => item.filename)).toEqual(['existing.pdf', 'Paket.eml', 'Rechnung.eml']);
    expect(feedback.loading).toHaveBeenCalledWith('Attaching messages…');
    expect(feedback.done).toHaveBeenCalled();
    expect(feedback.error).not.toHaveBeenCalled();
  });

  it('says so when a thread had no original to attach', async () => {
    harness.fetch.mockResolvedValue([eml('Paket.eml')]);
    const { attachments, feedback, drop } = setup();

    drop.onDrop(dragEvent({ [THREAD_DRAG_TYPE]: 'th-1,th-2' }));
    await settle();

    expect(attachments.value).toHaveLength(1);
    expect(feedback.error).toHaveBeenCalledWith(
      'The original of one message could not be loaded; it was not attached.',
    );
  });

  it('keeps the total attachment budget', async () => {
    harness.fetch.mockResolvedValue([eml('big.eml', MAX_TOTAL_ATTACHMENT_BYTES)]);
    const { attachments, feedback, drop } = setup([eml('existing.pdf', 10)]);

    drop.onDrop(dragEvent({ [THREAD_DRAG_TYPE]: 'th-1' }));
    await settle();

    expect(attachments.value.map((item) => item.filename)).toEqual(['existing.pdf']);
    expect(feedback.error).toHaveBeenCalledWith(expect.stringContaining('“big.eml” no longer fits'));
  });

  it('reports a failed fetch without touching the attachments', async () => {
    harness.fetch.mockRejectedValue({ key: 'backend.errors.unexpected', params: { message: 'offline' } });
    const { attachments, feedback, drop } = setup([eml('existing.pdf')]);

    drop.onDrop(dragEvent({ [THREAD_DRAG_TYPE]: 'th-1' }));
    await settle();

    expect(attachments.value).toHaveLength(1);
    expect(feedback.error).toHaveBeenCalledTimes(1);
    expect(drop.attaching.value).toBe(false);
  });

  it('leaves file drags to the drop zone and the editor', () => {
    const { drop } = setup();
    const event = dragEvent({ Files: '' });

    drop.onDragEnter(event);
    drop.onDragOver(event);
    drop.onDrop(event);

    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(event.stopPropagation).not.toHaveBeenCalled();
    expect(drop.dropActive.value).toBe(false);
    expect(harness.fetch).not.toHaveBeenCalled();
  });

  it('lights up while a thread hovers and offers a copy', () => {
    const { drop } = setup();
    const event = dragEvent({ [THREAD_DRAG_TYPE]: '' });

    drop.onDragEnter(event);
    drop.onDragEnter(event); // crossing into a child
    drop.onDragLeave(event);
    expect(drop.dropActive.value).toBe(true);
    drop.onDragOver(event);
    expect(event.dataTransfer?.dropEffect).toBe('copy');
    drop.onDragLeave(event);
    expect(drop.dropActive.value).toBe(false);
  });

  it('refuses drops while the message is sending', async () => {
    const { drop } = setup([], true);
    const event = dragEvent({ [THREAD_DRAG_TYPE]: 'th-1' });

    drop.onDragEnter(event);
    drop.onDragOver(event);
    expect(drop.dropActive.value).toBe(false);
    expect(event.dataTransfer?.dropEffect).toBe('none');
    drop.onDrop(event);
    await settle();
    expect(harness.fetch).not.toHaveBeenCalled();
  });
});
