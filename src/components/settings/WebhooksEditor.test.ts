// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendWebhook } from '@/services/backend';
import { answerConfirms } from '@/test/dialogs';

const backend = vi.hoisted(() => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendErrorKey: vi.fn(() => null),
  backendDeleteWebhook: vi.fn(),
  backendListWebhooks: vi.fn(async (): Promise<unknown[]> => []),
  backendUpsertWebhook: vi.fn(),
  translateBackendError: vi.fn(() => null),
}));
const toast = vi.hoisted(() => ({
  info: vi.fn(),
  success: vi.fn(),
  warning: vi.fn(),
  error: vi.fn(),
}));

vi.mock('@/services/backend', () => backend);
vi.mock('@/composables/useToast', () => ({ useToast: () => toast }));

import WebhooksEditor from './WebhooksEditor.vue';

/** A webhook forwards message metadata to a foreign host, so it must be removable. */
function webhook(overrides: Partial<BackendWebhook> = {}): BackendWebhook {
  return {
    id: 'hook-1',
    url: 'https://example.test/satchel',
    events_json: '["message.received","thread.updated"]',
    enabled: 1,
    created_at: '2026-08-01T10:00:00Z',
    updated_at: '2026-08-01T10:00:00Z',
    ...overrides,
  };
}

async function renderWithWebhook(entry: BackendWebhook = webhook()) {
  backend.backendListWebhooks.mockResolvedValue([entry]);
  const wrapper = mount(WebhooksEditor);
  await vi.waitFor(() => {
    expect(wrapper.find('.webhook-row').exists()).toBe(true);
  });
  return wrapper;
}

describe('webhooks in Accounts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    backend.backendDeleteWebhook.mockResolvedValue(undefined);
    backend.backendUpsertWebhook.mockResolvedValue(webhook());
  });

  it('names the subscribed events instead of showing a JSON string', async () => {
    const wrapper = await renderWithWebhook();

    expect(wrapper.find('.webhook-events').text()).toBe('message.received · thread.updated');
    expect(wrapper.find('.webhook-row').text()).not.toContain('["message.received"');
  });

  it('keeps the row readable when the stored event list is broken', async () => {
    const wrapper = await renderWithWebhook(webhook({ events_json: 'not json' }));

    expect(wrapper.find('.webhook-url').text()).toBe('https://example.test/satchel');
    expect(wrapper.find('.webhook-events').text()).toBe('no events');
  });

  it('deletes nothing when the confirmation is declined', async () => {
    const confirmSpy = answerConfirms(false);
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row button').trigger('click');

    expect(backend.backendDeleteWebhook).not.toHaveBeenCalled();
    expect(wrapper.findAll('.webhook-row')).toHaveLength(1);
    confirmSpy.stop();
  });

  it('names the target URL in the question and deletes the confirmed webhook once', async () => {
    const confirmSpy = answerConfirms(true);
    const wrapper = await renderWithWebhook(webhook({ id: 'hook-7' }));

    await wrapper.find('.webhook-row button').trigger('click');
    await vi.waitFor(() => {
      expect(backend.backendDeleteWebhook).toHaveBeenCalledTimes(1);
    });

    expect(confirmSpy.asked.mock.calls[0]?.[0]?.body ?? '').toContain('https://example.test/satchel');
    expect(backend.backendDeleteWebhook).toHaveBeenCalledWith('hook-7');
    confirmSpy.stop();
  });

  it('keeps the row and raises a toast when the deletion fails', async () => {
    const confirmSpy = answerConfirms(true);
    backend.backendDeleteWebhook.mockRejectedValue(new Error('database is locked'));
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row button').trigger('click');
    await vi.waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith('Could not delete webhook: database is locked');
    });

    expect(wrapper.findAll('.webhook-row')).toHaveLength(1);
    confirmSpy.stop();
  });

  it('switches a webhook off through the upsert', async () => {
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row input[type="checkbox"]').setValue(false);
    await vi.waitFor(() => {
      expect(backend.backendUpsertWebhook).toHaveBeenCalledTimes(1);
    });

    expect(backend.backendUpsertWebhook).toHaveBeenCalledWith({
      id: 'hook-1',
      url: 'https://example.test/satchel',
      events: ['message.received', 'thread.updated'],
      enabled: false,
    });
  });

  it('puts the switch back when the backend refuses', async () => {
    backend.backendUpsertWebhook.mockRejectedValue(new Error('webhook store unavailable'));
    const wrapper = await renderWithWebhook();

    await wrapper.find('.webhook-row input[type="checkbox"]').setValue(false);
    await vi.waitFor(() => {
      expect(toast.error).toHaveBeenCalledWith(
        'Could not switch webhook: webhook store unavailable',
      );
    });

    const toggle = wrapper.find('.webhook-row input[type="checkbox"]')
      .element as HTMLInputElement;
    expect(toggle.checked).toBe(true);
  });
});
