// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { answerConfirms, type DialogStub } from '@/test/dialogs';
import type { ConfirmRequest } from '@/composables/useConfirm';
import UnsubscribeBanner from './UnsubscribeBanner.vue';

const unsubscribe = vi.fn();
const toastError = vi.fn();
const toastSuccess = vi.fn();

vi.mock('@/stores/messages', () => ({
  useMessagesStore: () => ({ unsubscribe }),
}));

vi.mock('@/composables/useToast', () => ({
  useToast: () => ({ error: toastError, success: toastSuccess }),
}));

const unsubscribeButton = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('button').find((button) => button.text() === 'Unsubscribe')!;

describe('unsubscribe banner', () => {
  let confirms: DialogStub<ConfirmRequest, boolean>;

  beforeEach(() => {
    confirms = answerConfirms(true);
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  afterEach(() => confirms.stop());

  it('names the refused target instead of showing the serialized error', async () => {
    unsubscribe.mockRejectedValue(
      new Error(JSON.stringify({ key: 'backend.errors.unsubscribeTargetBlocked' })),
    );
    const wrapper = mount(UnsubscribeBanner, { props: { messageId: 'msg-1' } });

    await unsubscribeButton(wrapper).trigger('click');
    await Promise.resolve();

    expect(toastError).toHaveBeenCalledTimes(1);
    const message = String(toastError.mock.calls[0]?.[0]);
    expect(message).toContain('not publicly reachable');
    expect(message).not.toContain('backend.errors');
  });

  it('reports the route a successful unsubscribe took', async () => {
    unsubscribe.mockResolvedValue({ method: 'http-one-click', target: 'https://list.example/u', confirm_mail: null });
    const wrapper = mount(UnsubscribeBanner, { props: { messageId: 'msg-1' } });

    await unsubscribeButton(wrapper).trigger('click');
    await flushPromises();

    expect(confirms.asked).not.toHaveBeenCalled();
    expect(toastSuccess).toHaveBeenCalledWith('Unsubscribed via http-one-click');
  });

  it('shows the exact mail and sends it only once confirmed', async () => {
    const mail = { to: 'leave@list.example', subject: 'unsubscribe 1234', body: 'please\nremove' };
    unsubscribe
      .mockResolvedValueOnce({ method: 'mailto', target: mail.to, confirm_mail: mail })
      .mockResolvedValueOnce({ method: 'mailto', target: mail.to, confirm_mail: null });
    const wrapper = mount(UnsubscribeBanner, { props: { messageId: 'msg-1' } });

    await unsubscribeButton(wrapper).trigger('click');
    await flushPromises();

    expect(confirms.asked).toHaveBeenCalledTimes(1);
    expect(confirms.asked.mock.calls[0]![0].fields?.map((field) => field.value))
      .toEqual(['leave@list.example', 'unsubscribe 1234', 'please\nremove']);
    expect(unsubscribe).toHaveBeenNthCalledWith(1, 'msg-1');
    expect(unsubscribe).toHaveBeenNthCalledWith(2, 'msg-1', mail);
    expect(toastSuccess).toHaveBeenCalledWith('Unsubscribed via mailto');
  });

  it('sends nothing and stays open when the mail is declined', async () => {
    confirms.reply(false);
    unsubscribe.mockResolvedValueOnce({
      method: 'mailto', target: 'leave@list.example',
      confirm_mail: { to: 'leave@list.example', subject: 'unsubscribe', body: 'unsubscribe' },
    });
    const wrapper = mount(UnsubscribeBanner, { props: { messageId: 'msg-1' } });

    await unsubscribeButton(wrapper).trigger('click');
    await flushPromises();

    expect(unsubscribe).toHaveBeenCalledTimes(1);
    expect(toastSuccess).not.toHaveBeenCalled();
    expect(wrapper.find('.unsub').exists()).toBe(true);
  });
});
