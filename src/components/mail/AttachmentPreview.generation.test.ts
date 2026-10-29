// @vitest-environment jsdom
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { t } from '@/i18n/runtime';
import AttachmentPreview from './AttachmentPreview.vue';

const harness = vi.hoisted(() => ({ download: vi.fn(), open: vi.fn(), error: vi.fn() }));
vi.mock('@/composables/useAttachmentActions', () => ({ useAttachmentActions: () => ({
  downloadAttachment: harness.download, openAttachmentDefault: harness.open,
  nestedMessagePath: () => null,
}) }));
vi.mock('@/composables/useToast', () => ({ useToast: () => ({ error: harness.error }) }));
let wrapper: VueWrapper | null = null;
beforeEach(() => { setActivePinia(createPinia()); vi.clearAllMocks(); });
afterEach(() => { wrapper?.unmount(); wrapper = null; document.body.innerHTML = ''; });

it.each(['imapUidValidityChanged', 'imapUidValidityMissing'])('shows folder recovery when preview fetch fails with %s', async (key) => {
  harness.download.mockRejectedValue({ key: `backend.errors.${key}`, params: { folder: 'INBOX' } });
  wrapper = mount(AttachmentPreview, { attachTo: document.body, props: { attachmentId: 'attachment' } });
  await flushPromises();
  expect(document.body.textContent).toContain('INBOX');
  expect(document.body.textContent).toMatch(/sync.*folder/i);
  expect(document.body.textContent).not.toContain('[object Object]');
});

it.each(['imapUidValidityChanged', 'imapUidValidityMissing'])('shows folder recovery when external open fails with %s', async (key) => {
  harness.download.mockResolvedValue({ filename: 'file.bin', mime_type: 'application/octet-stream', data_base64: 'QQ==' });
  harness.open.mockRejectedValue({ key: `backend.errors.${key}`, params: { folder: 'INBOX' } });
  wrapper = mount(AttachmentPreview, { attachTo: document.body, props: { attachmentId: 'attachment' } });
  await flushPromises();
  const button = [...document.body.querySelectorAll('button')].find(element => element.textContent?.trim() === t('mail.attachment.openWithDefault'));
  expect(button).toBeDefined();
  button!.click();
  await flushPromises();
  const message = harness.error.mock.calls[0]![0] as string;
  expect(message).toContain('INBOX');
  expect(message).toMatch(/sync.*folder/i);
});
