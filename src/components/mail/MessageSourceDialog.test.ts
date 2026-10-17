// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import MessageSourceDialog from './MessageSourceDialog.vue';

const loadMessageSource = vi.hoisted(() => vi.fn());

vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({ loadMessageSource }),
}));

let wrapper: VueWrapper | null = null;

beforeEach(() => {
  setActivePinia(createPinia());
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = '';
  vi.clearAllMocks();
});

describe('message source dialog', () => {
  it('opens on the original RFC 822 source when the provider returns it', async () => {
    loadMessageSource.mockResolvedValue({
      message_id: 'message-1',
      subject: 'Signed message',
      raw_source: 'From: sender@example.com\r\nContent-Type: text/plain\r\n\r\nHello',
      raw_source_status: 'loaded',
      raw_headers: 'From: sender@example.com\r\nContent-Type: text/plain',
      body_text: 'Hello',
      raw_size: 64,
    });

    wrapper = mount(MessageSourceDialog, {
      attachTo: document.body,
      props: { open: true, messageId: 'message-1' },
    });
    await flushPromises();

    const dialog = document.body.querySelector('[role="dialog"]');
    expect(dialog?.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toContain('Original');
    expect(dialog?.querySelector('pre')?.textContent).toContain('Content-Type: text/plain');
    expect(dialog?.querySelector('.source-notice')).toBeNull();
  });

  it('keeps decoded content available when the original cannot be fetched', async () => {
    loadMessageSource.mockResolvedValue({
      message_id: 'message-2',
      subject: 'Stored message',
      raw_source: null,
      raw_source_status: 'unavailable',
      body_text: 'Stored body',
    });

    wrapper = mount(MessageSourceDialog, {
      attachTo: document.body,
      props: { open: true, messageId: 'message-2' },
    });
    await flushPromises();

    const dialog = document.body.querySelector('[role="dialog"]');
    expect(dialog?.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toContain('Text');
    expect(dialog?.querySelector('pre')?.textContent).toContain('Stored body');
    expect(dialog?.querySelector('.source-notice')?.textContent).toContain(
      'could not be fetched from the provider',
    );
  });
});
