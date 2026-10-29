// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';

const backend = vi.hoisted(() => ({
  openMessageFile: vi.fn(),
  listAccounts: vi.fn(),
  readAttachment: vi.fn(),
  importMbox: vi.fn(),
}));

// The viewer imports the submodules, not the barrel — a barrel import pulls
// the whole service surface into this window's bundle (PERF-08).
vi.mock('@/services/backend/system', () => ({
  backendOpenMessageFile: backend.openMessageFile,
  backendReadMessageFileAttachment: backend.readAttachment,
}));
vi.mock('@/services/backend/accounts', () => ({
  backendListAccounts: backend.listAccounts,
}));
vi.mock('@/services/backend/backup', () => ({
  backendImportMbox: backend.importMbox,
}));

vi.mock('@/components/mail/EmailRenderer.vue', () => ({
  default: { template: '<div class="renderer-stub" />' },
}));

import MessageFileViewer from './MessageFileViewer.vue';
import { viewerI18n, viewerI18nReady } from '@/i18n/viewer';
import { setActiveI18n } from '@/i18n/runtime';

function encodePath(path: string): string {
  const bytes = new TextEncoder().encode(path);
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_');
}

function mountViewer() {
  return mount(MessageFileViewer, { global: { plugins: [createPinia()] } });
}

beforeEach(async () => {
  // The viewer catalogue loads lazily; translations must exist before mounting.
  await viewerI18nReady;
  vi.clearAllMocks();
  vi.stubGlobal('matchMedia', vi.fn(() => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })));
  // Backend errors are translated through the runtime composer, and in this
  // window that is the viewer subset — not the client catalogue.
  setActiveI18n(viewerI18n.global as never);
  window.history.replaceState({}, '', `/viewer.html?p=${encodePath('/mnt/share/mail.eml')}`);
  backend.listAccounts.mockResolvedValue([]);
});

describe('message file viewer', () => {
  it('offers inline parts for saving when their bytes cannot be embedded', async () => {
    backend.openMessageFile.mockResolvedValue({
      subject: 'Large inline photo',
      from_address: 'sender@example.test',
      to_addresses: [],
      cc_addresses: [],
      date_rfc3339: null,
      body_html: '<img src="cid:large-photo">',
      body_text: null,
      attachments: [
        { index: 0, filename: 'large-photo.jpg', content_type: 'image/jpeg', size: 24_403_635,
          content_id: 'large-photo', is_inline: true, data_url: null },
        { index: 1, filename: 'logo.png', content_type: 'image/png', size: 4,
          content_id: 'logo', is_inline: true, data_url: 'data:image/png;base64,AQIDBA==' },
      ],
    });

    const wrapper = mountViewer();
    await flushPromises();

    expect(wrapper.text()).toContain('large-photo.jpg');
    expect(wrapper.text()).not.toContain('logo.png');
    expect(wrapper.findAll('button').filter((button) => button.text().includes('Save'))).toHaveLength(1);
  });

  it('translates a command error instead of showing the key', async () => {
    backend.openMessageFile.mockRejectedValue({
      key: 'backend.errors.unexpected',
      params: { message: 'cannot read /mnt/share/mail.eml' },
    });

    const wrapper = mountViewer();
    await flushPromises();

    const description = wrapper.get('.empty p').text();
    expect(description).not.toContain('backend.errors.');
    expect(description.startsWith('Error: ')).toBe(false);
    expect(description).toBe('An unexpected backend error occurred.');
  });

  it('falls back to the generic failure for a key the viewer does not ship', async () => {
    backend.openMessageFile.mockRejectedValue(
      new Error(JSON.stringify({ key: 'backend.errors.webhookUrlInvalid' })),
    );

    const wrapper = mountViewer();
    await flushPromises();

    const description = wrapper.get('.empty p').text();
    expect(description).toBe('An unexpected backend error occurred.');
  });

  it('shows a plain failure without the Error prefix', async () => {
    backend.openMessageFile.mockRejectedValue(new Error('window handle is gone'));

    const wrapper = mountViewer();
    await flushPromises();

    expect(wrapper.get('.empty p').text()).toBe('window handle is gone');
  });
});

/*
 * The window used to be a flat page with a rule under the header, while the
 * same mail in the client sits as a card on an inset canvas (UI-17). Two
 * shapes for one message is the kind of thing nobody reports and everybody
 * notices.
 */
describe('the reading canvas', () => {
  it('lays the message body in a card', async () => {
    backend.openMessageFile.mockResolvedValue({
      subject: 'Rechnung',
      from_address: 'billing@example.com',
      from_name: 'Billing',
      to_addresses: [],
      cc_addresses: [],
      attachments: [],
      body_html: '<p>hi</p>',
      body_text: null,
      date_rfc3339: null,
    });

    const wrapper = mountViewer();
    await flushPromises();

    expect(wrapper.find('.card .renderer-stub').exists()).toBe(true);
  });

  it('waits with the empty state from the kit rather than a bare line of text', () => {
    backend.openMessageFile.mockReturnValue(new Promise(() => {}));

    const wrapper = mountViewer();

    expect(wrapper.find('.empty').exists()).toBe(true);
    expect(wrapper.find('.state').exists()).toBe(false);
  });
});
