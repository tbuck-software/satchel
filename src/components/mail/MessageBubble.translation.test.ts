// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import type { Message } from '@/types/mail';
import type { BackendTranslationSettings } from '@/services/backend/translation';
import { useSettingsStore } from '@/stores/settings';
import { useUiStore } from '@/stores/ui';
import { useThreadsStore } from '@/stores/threads';
import { useTranslationStore } from '@/stores/translation';
import { toggleMessageTranslation } from '@/composables/useMessageTranslation';
import { useToast } from '@/composables/useToast';
import MessageBubble from './MessageBubble.vue';
import EmailRenderer from './EmailRenderer.vue';

/*
 * Translate in the reader: offered from the header, the translated body
 * replaces the original in the same renderer, and a muted line leads back.
 * Consent comes first for an account that has not given it.
 */

const backend = vi.hoisted(() => ({
  offer: vi.fn(),
  translate: vi.fn(),
  consent: vi.fn(),
}));
vi.mock('@/services/backend/translation', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend/translation')>(),
  backendMessageTranslationOffer: backend.offer,
  backendTranslateMessage: backend.translate,
  backendSetTranslationConsent: backend.consent,
}));

const ask = vi.hoisted(() => vi.fn());
vi.mock('@/composables/useConfirm', () => ({ useConfirm: () => ({ ask }) }));

let wrapper: VueWrapper | null = null;

function message(): Message {
  return {
    id: 'm-1',
    threadId: 'thread-1',
    from: { name: 'Ana', email: 'ana@example.test' },
    to: [{ name: 'Tom', email: 'tom@example.com' }],
    subject: 'Reunión',
    bodyText: 'Hola a todos, os envío los documentos para la reunión del lunes.',
    bodyHtml: '<p>Hola a todos, os envío los documentos para la reunión del lunes.</p>',
    bodyLoaded: true,
    timestamp: '2026-09-03T10:00:03Z',
    isRead: true,
    attachments: [],
  };
}

function settings(consented: boolean): BackendTranslationSettings {
  return {
    engine: 'deepl',
    target_language: 'DE',
    log_sent_text: false,
    engines: [{ id: 'deepl', has_key: true, plan: 'pro', local: false }],
    consents: consented ? [{ engine: 'deepl', account_id: 'acc-1', granted_at: '2026-11-06T00:00:00Z' }] : [],
  };
}

const TRANSLATION = {
  message_id: 'm-1',
  target_language: 'DE',
  engine: 'deepl',
  source_language: 'ES',
  body_html: '<p>Hallo zusammen, anbei die Unterlagen für Montag.</p>',
  body_text: null,
  cached: false,
  created_at: '2026-11-06T10:00:00Z',
};

async function render(consented = true, collapsed = false) {
  useTranslationStore().apply(settings(consented));
  vi.spyOn(useThreadsStore(), 'find').mockReturnValue({ id: 'thread-1', accountId: 'acc-1' } as never);
  wrapper = mount(MessageBubble, {
    attachTo: document.body,
    props: { message: message(), canReply: true, collapsed },
    global: { stubs: { SenderFilingMenu: true } },
  });
  await flushPromises();
  return wrapper;
}

beforeEach(() => {
  setActivePinia(createPinia());
  useSettingsStore().theme = 'light';
  useUiStore().isMobile = false;
  vi.stubGlobal('requestAnimationFrame', vi.fn(() => 1));
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  backend.offer.mockResolvedValue({
    detected_language: 'ES', offered: true, cached: false, cached_source_language: null, consented: true, refusal: null,
  });
  backend.translate.mockResolvedValue(TRANSLATION);
  backend.consent.mockImplementation(async () => settings(true));
  ask.mockResolvedValue(true);
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  document.body.innerHTML = '';
});

describe('translating a message', () => {
  it('renders the translation in the same renderer and leads back to the original', async () => {
    const view = await render();
    expect(backend.offer).toHaveBeenCalledWith('m-1', 'DE');

    const trigger = view.get('.message-translate-trigger');
    expect(trigger.attributes('aria-label')).toBe('Translate');
    await trigger.trigger('click');
    await flushPromises();

    expect(backend.translate).toHaveBeenCalledWith('m-1', 'DE');
    expect(ask).not.toHaveBeenCalled();
    const renderer = view.getComponent(EmailRenderer);
    expect(renderer.props('html')).toBe(TRANSLATION.body_html);
    expect(String(renderer.props('cacheKey'))).toContain(':translated:DE');
    const note = view.get('.translation-note');
    expect(note.text()).toContain('Translated from');
    expect(note.text()).toContain('Spanish');

    await note.get('button').trigger('click');
    await flushPromises();
    expect(view.getComponent(EmailRenderer).props('html')).toBe(message().bodyHtml);
    expect(view.find('.translation-note').exists()).toBe(false);

    // Back again without a second request: the translation is kept.
    expect(toggleMessageTranslation('m-1')).toBe(true);
    await flushPromises();
    expect(view.getComponent(EmailRenderer).props('html')).toBe(TRANSLATION.body_html);
    expect(backend.translate).toHaveBeenCalledTimes(1);
  });

  it('asks before the first message of an account leaves the device', async () => {
    ask.mockResolvedValueOnce(false);
    const view = await render(false);
    await view.get('.message-translate-trigger').trigger('click');
    await flushPromises();
    expect(ask).toHaveBeenCalledTimes(1);
    expect(backend.consent).not.toHaveBeenCalled();
    expect(backend.translate).not.toHaveBeenCalled();

    await view.get('.message-translate-trigger').trigger('click');
    await flushPromises();
    expect(backend.consent).toHaveBeenCalledWith('deepl', 'acc-1', true);
    expect(backend.translate).toHaveBeenCalledTimes(1);
  });

  it('names the plan when it asks', async () => {
    ask.mockResolvedValueOnce(false);
    const view = await render(false);
    await view.get('.message-translate-trigger').trigger('click');
    await flushPromises();
    const question = ask.mock.calls[0]![0] as { body: string; detail: string };
    expect(question.body).toContain('DeepL API Pro');
    expect(question.detail).toContain('72 hours');
  });

  it('says why instead of asking when the message is refused', async () => {
    backend.offer.mockResolvedValue({
      detected_language: 'ES', offered: false, cached: false, cached_source_language: null, consented: false,
      refusal: 'backend.errors.translation.encrypted',
    });
    const view = await render(false);
    expect(view.find('.message-translate-trigger').exists()).toBe(false);
    expect(toggleMessageTranslation('m-1')).toBe(true);
    await flushPromises();
    expect(ask).not.toHaveBeenCalled();
    expect(backend.translate).not.toHaveBeenCalled();
    expect(useToast().state.toasts.at(-1)?.message).toBe('Encrypted mail is not sent for translation.');
  });

  it('asks nothing for a folded message and leaves it to T only when open', async () => {
    const view = await render(true, true);
    expect(backend.offer).not.toHaveBeenCalled();
    expect(toggleMessageTranslation('m-1')).toBe(false);

    await view.setProps({ collapsed: false });
    await flushPromises();
    expect(backend.offer).toHaveBeenCalledWith('m-1', 'DE');
    expect(toggleMessageTranslation('m-1')).toBe(true);
  });

  it('keeps T for mail that looks like the target language, without a header button', async () => {
    backend.offer.mockResolvedValue({
      detected_language: 'DE', offered: false, cached: false, cached_source_language: null, consented: true, refusal: null,
    });
    const view = await render();
    expect(view.find('.message-translate-trigger').exists()).toBe(false);
    // The guess can be wrong; asked for, it translates anyway.
    expect(toggleMessageTranslation('m-1')).toBe(true);
    await flushPromises();
    expect(backend.translate).toHaveBeenCalledWith('m-1', 'DE');
    expect(view.getComponent(EmailRenderer).props('html')).toBe(TRANSLATION.body_html);
  });

  it('offers nothing with a DeepL API Free key, for any account', async () => {
    useTranslationStore().apply({
      ...settings(true),
      engines: [{ id: 'deepl', has_key: true, plan: 'free', local: false }],
    });
    wrapper = mount(MessageBubble, {
      attachTo: document.body,
      props: { message: message(), canReply: true },
      global: { stubs: { SenderFilingMenu: true } },
    });
    await flushPromises();
    expect(backend.offer).not.toHaveBeenCalled();
    expect(wrapper.find('.message-translate-trigger').exists()).toBe(false);
    expect(toggleMessageTranslation('m-1')).toBe(false);
  });

  it('offers nothing while no engine is set up', async () => {
    useTranslationStore().apply({ ...settings(true), engine: null });
    wrapper = mount(MessageBubble, {
      attachTo: document.body,
      props: { message: message(), canReply: true },
      global: { stubs: { SenderFilingMenu: true } },
    });
    await flushPromises();
    expect(backend.offer).not.toHaveBeenCalled();
    expect(wrapper.find('.message-translate-trigger').exists()).toBe(false);
  });
});
