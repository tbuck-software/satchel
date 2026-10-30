// @vitest-environment jsdom
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { t } from '@/i18n/runtime';
import AttachmentPreview from './AttachmentPreview.vue';
import * as payloads from '@/utils/attachmentBytes';

const harness = vi.hoisted(() => ({ download: vi.fn(), open: vi.fn(), error: vi.fn() }));
vi.mock('@/composables/useAttachmentActions', () => ({ useAttachmentActions: () => ({
  downloadAttachment: harness.download, openAttachmentDefault: harness.open,
  nestedMessagePath: () => null,
}) }));
vi.mock('@/composables/useToast', () => ({ useToast: () => ({ error: harness.error }) }));
let wrapper: VueWrapper | null = null;
beforeEach(() => { setActivePinia(createPinia()); vi.clearAllMocks(); });
afterEach(() => { wrapper?.unmount(); wrapper = null; document.body.innerHTML = ''; vi.restoreAllMocks(); vi.unstubAllGlobals(); });

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

it('does not allocate a blob when a download resolves after unmount', async () => {
  const create = vi.fn(() => 'blob:late');
  const revoke = vi.fn();
  vi.stubGlobal('URL', { createObjectURL: create, revokeObjectURL: revoke });
  let resolve!: (value: unknown) => void;
  harness.download.mockReturnValue(new Promise(done => { resolve = done; }));
  wrapper = mount(AttachmentPreview, { props: { attachmentId: 'old' } });
  wrapper.unmount(); wrapper = null;
  resolve({ filename: 'old.png', mime_type: 'image/png', data_base64: 'QQ==' });
  await flushPromises();
  expect(create).not.toHaveBeenCalled();
  vi.unstubAllGlobals();
});

it('replaces equal-length images without letting an older request revoke the current URL', async () => {
  const create = vi.fn().mockReturnValueOnce('blob:new').mockReturnValueOnce('blob:replacement');
  const revoke = vi.fn();
  vi.stubGlobal('URL', { createObjectURL: create, revokeObjectURL: revoke });
  let resolve!: (value: unknown) => void;
  harness.download.mockReturnValueOnce(new Promise(done => { resolve = done; }))
    .mockResolvedValueOnce({ filename: 'new.png', mime_type: 'image/png', data_base64: 'Qg==' })
    .mockResolvedValueOnce({ filename: 'replacement.png', mime_type: 'image/png', data_base64: 'Qw==' });
  wrapper = mount(AttachmentPreview, { props: { attachmentId: 'old' } });
  await wrapper.setProps({ attachmentId: 'new' });
  await flushPromises();
  expect(wrapper.find('img').attributes('src')).toBe('blob:new');
  resolve({ filename: 'old.png', mime_type: 'image/png', data_base64: 'QQ==' });
  await flushPromises();
  expect(revoke).not.toHaveBeenCalled();
  await wrapper.setProps({ attachmentId: 'replacement' });
  await flushPromises();
  expect(revoke).toHaveBeenCalledExactlyOnceWith('blob:new');
  expect(wrapper.find('img').attributes('src')).toBe('blob:replacement');
  wrapper.unmount(); wrapper = null;
  expect(revoke).toHaveBeenLastCalledWith('blob:replacement');
  vi.unstubAllGlobals();
});

it('ignores a delayed cache-file read after the preview is replaced or unmounted', async () => {
  const create = vi.fn(() => 'blob:current');
  const revoke = vi.fn();
  vi.stubGlobal('URL', { createObjectURL: create, revokeObjectURL: revoke });
  let release!: (value: Uint8Array) => void;
  vi.spyOn(payloads, 'attachmentBytes').mockReturnValueOnce(new Promise(done => { release = done; }))
    .mockResolvedValueOnce(new Uint8Array([2]));
  harness.download.mockResolvedValue({ filename: 'image.png', mime_type: 'image/png', cached_path: '/cache/image' });
  wrapper = mount(AttachmentPreview, { props: { attachmentId: 'old' } });
  await flushPromises();
  await wrapper.setProps({ attachmentId: 'current' });
  await flushPromises();
  expect(wrapper.find('img').attributes('src')).toBe('blob:current');
  wrapper.unmount(); wrapper = null;
  release(new Uint8Array([1]));
  await flushPromises();
  expect(create).toHaveBeenCalledTimes(1);
  expect(revoke).toHaveBeenCalledExactlyOnceWith('blob:current');
});
