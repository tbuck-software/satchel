// @vitest-environment jsdom
import { defineComponent } from 'vue';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useSettingsStore } from '@/stores/settings';
import { useAttachmentOpen } from './useAttachmentOpen';

const harness = vi.hoisted(() => ({ download: vi.fn(), open: vi.fn(), quickLook: vi.fn(), error: vi.fn() }));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendDownloadAttachment: harness.download,
  backendOpenAttachmentDefault: harness.open,
  backendToggleAttachmentQuickLook: harness.quickLook,
}));
vi.mock('@/composables/useToast', () => ({ useToast: () => ({ error: harness.error }) }));
let wrapper: VueWrapper | null = null;
beforeEach(() => { setActivePinia(createPinia()); vi.clearAllMocks(); });
afterEach(() => { wrapper?.unmount(); wrapper = null; });

it.each([
  ['save', 'imapUidValidityChanged'], ['save', 'imapUidValidityMissing'],
  ['open', 'imapUidValidityChanged'], ['open', 'imapUidValidityMissing'],
  ['quickLook', 'imapUidValidityChanged'], ['quickLook', 'imapUidValidityMissing'],
])('shows folder recovery when %s fails with %s', async (action, key) => {
  const error = { key: `backend.errors.${key}`, params: { folder: 'INBOX' } };
  harness.download.mockRejectedValue(error);
  harness.open.mockRejectedValue(error);
  harness.quickLook.mockRejectedValue(error);
  const settings = useSettingsStore();
  settings.attachmentOpenBehavior = 'system';
  settings.keybindings.previewAttachment = 'space';
  let open!: ReturnType<typeof useAttachmentOpen>;
  wrapper = mount(defineComponent({ setup() {
    open = useAttachmentOpen(() => ({ id: 'attachment', filename: 'report.pdf', contentType: 'application/pdf', size: 12 }));
    return () => null;
  } }));
  if (action === 'save') await open.save();
  else if (action === 'open') open.openPrimary();
  else open.handlePreviewKey(new KeyboardEvent('keydown', { key: ' ', code: 'Space' }));
  await flushPromises();
  expect(harness.error).toHaveBeenCalledOnce();
  const message = harness.error.mock.calls[0]![0] as string;
  expect(message).toContain('INBOX');
  expect(message).toMatch(/sync.*folder/i);
  expect(message).not.toContain('[object Object]');
});
