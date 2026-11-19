// @vitest-environment jsdom
import { createPinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, expect, it, vi } from 'vitest';
import { i18n } from '@/i18n';
import type { BackendDownloadAttachmentResult } from '@/types/messages';
import AttachmentCard from './AttachmentCard.vue';

const { downloadAttachment } = vi.hoisted(() => ({ downloadAttachment: vi.fn() }));
vi.mock('@/composables/useAttachmentActions', () => ({
  useAttachmentActions: () => ({ downloadAttachment }),
}));

afterEach(() => {
  vi.unstubAllGlobals();
  downloadAttachment.mockReset();
});

it('does not create a thumbnail URL for a download that arrives after unmount', async () => {
  let resolveDownload!: (data: BackendDownloadAttachmentResult) => void;
  const download = new Promise<BackendDownloadAttachmentResult>((resolve) => { resolveDownload = resolve; });
  const createObjectURL = vi.fn(() => 'blob:thumbnail');
  downloadAttachment.mockReturnValue(download);
  vi.stubGlobal('IntersectionObserver', undefined);
  vi.stubGlobal('URL', class extends URL {
    static override createObjectURL = createObjectURL;
  });

  const card = mount(AttachmentCard, {
    props: { attachment: { id: 'picture', contentType: 'image/png', filename: 'picture.png', size: 4 } },
    global: { plugins: [createPinia(), i18n], stubs: { AttachmentPreview: true } },
  });
  await flushPromises();
  expect(downloadAttachment).toHaveBeenCalledExactlyOnceWith('picture');
  card.unmount();

  resolveDownload({ filename: 'picture.png', mime_type: 'image/png', data_base64: 'AA==' });
  await flushPromises();
  await vi.dynamicImportSettled();
  expect(createObjectURL).not.toHaveBeenCalled();
});
