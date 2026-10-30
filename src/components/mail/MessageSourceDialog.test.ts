// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import MessageSourceDialog from './MessageSourceDialog.vue';

const loadMessageSource = vi.hoisted(() => vi.fn());
const exportMessage = vi.hoisted(() => vi.fn().mockResolvedValue(false));

vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({ loadMessageSource, exportMessage }),
}));

let wrapper: VueWrapper | null = null;

function source(raw: string, messageId = 'message-1') {
  return { message_id: messageId, subject: raw, raw_source: raw, raw_source_status: 'loaded', raw_size: raw.length };
}

function deferredSource() {
  let resolve!: (value: ReturnType<typeof source>) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<ReturnType<typeof source>>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function exportButton() {
  return [...document.body.querySelectorAll('button')].find(element => element.textContent?.includes('Save as EML'))!;
}

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
  it.each(['before', 'after'])('ignores a closed request completing %s the reopened request', async (order) => {
    const old = deferredSource();
    const current = deferredSource();
    loadMessageSource.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    wrapper = mount(MessageSourceDialog, { attachTo: document.body, props: { open: true, messageId: 'message-1' } });
    await wrapper.setProps({ open: false });
    await wrapper.setProps({ open: true });
    expect(loadMessageSource).toHaveBeenCalledTimes(2);

    if (order === 'before') {
      old.resolve(source('Old source'));
      await flushPromises();
      expect(exportButton().disabled).toBe(true);
      expect(document.body.querySelector('pre')).toBeNull();
    }
    current.resolve(source('Current source'));
    await flushPromises();
    if (order === 'after') {
      old.resolve(source('Old source'));
      await flushPromises();
    }
    expect(document.body.querySelector('pre')?.textContent).toBe('Current source');
    expect(exportButton().disabled).toBe(false);
  });

  it('ignores an old error and finalizer while retaining the current request error', async () => {
    const old = deferredSource();
    const current = deferredSource();
    loadMessageSource.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    wrapper = mount(MessageSourceDialog, { attachTo: document.body, props: { open: true, messageId: 'message-1' } });
    await wrapper.setProps({ open: false });
    await wrapper.setProps({ open: true });
    old.reject(new Error('Old failure'));
    await flushPromises();
    expect(document.body.querySelector('[role="alert"]')).toBeNull();
    expect(exportButton().disabled).toBe(true);
    current.reject(new Error('Current failure'));
    await flushPromises();
    expect(document.body.querySelector('[role="alert"]')?.textContent).toBe('Current failure');
    expect(exportButton().disabled).toBe(false);
  });

  it('clears all previous source details when switching messages', async () => {
    const current = deferredSource();
    loadMessageSource.mockResolvedValueOnce(source('Previous subject')).mockReturnValueOnce(current.promise);
    wrapper = mount(MessageSourceDialog, { attachTo: document.body, props: { open: true, messageId: 'message-1' } });
    await flushPromises();
    await wrapper.setProps({ messageId: 'message-2' });
    expect(document.body.querySelector('.subject')).toBeNull();
    expect(document.body.querySelector('.size')).toBeNull();
    expect(document.body.querySelector('pre')).toBeNull();
    expect(exportButton().disabled).toBe(true);
    current.resolve(source('New source', 'message-2'));
    await flushPromises();
    expect(document.body.querySelector('pre')?.textContent).toBe('New source');
  });

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

  it('saves an EML through the original-message export command', async () => {
    loadMessageSource.mockResolvedValue({ message_id: 'message-1', subject: 'Original', raw_source_status: 'loaded', raw_source: 'Original MIME' });
    wrapper = mount(MessageSourceDialog, { attachTo: document.body, props: { open: true, messageId: 'message-1' } });
    await flushPromises();
    const button = [...document.body.querySelectorAll('button')].find(element => element.textContent?.includes('Save as EML'))!;
    button.click();
    await flushPromises();
    expect(exportMessage).toHaveBeenCalledExactlyOnceWith('message-1', 'Original');
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
