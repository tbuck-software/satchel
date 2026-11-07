// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import type { BackendTranslationSettings } from '@/services/backend/translation';
import { useAccountsStore } from '@/stores/accounts';
import { useTranslationStore } from '@/stores/translation';
import TranslationSettings from './TranslationSettings.vue';

const backend = vi.hoisted(() => ({
  setKey: vi.fn(),
  check: vi.fn(),
  save: vi.fn(),
  consent: vi.fn(),
  languages: vi.fn(),
  log: vi.fn(),
  logText: vi.fn(),
}));
vi.mock('@/services/backend/translation', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend/translation')>(),
  backendSetTranslationApiKey: backend.setKey,
  backendCheckTranslationEngine: backend.check,
  backendSaveTranslationSettings: backend.save,
  backendSetTranslationConsent: backend.consent,
  backendListTranslationLanguages: backend.languages,
  backendListOutboundRequests: backend.log,
  backendOutboundRequestText: backend.logText,
}));

function settings(patch: Partial<BackendTranslationSettings> = {}): BackendTranslationSettings {
  return {
    engine: null,
    target_language: null,
    log_sent_text: false,
    engines: [{ id: 'deepl', has_key: false, plan: null, local: false }],
    consents: [],
    ...patch,
  };
}

let wrapper: VueWrapper | null = null;

beforeEach(() => {
  setActivePinia(createPinia());
  useTranslationStore().apply(settings());
  useAccountsStore().accounts = [
    { id: 'acc-1', displayName: 'Private', email: 'me@example.com', provider: 'imap', kind: 'email', color: '#000000' },
  ] as never;
  backend.languages.mockResolvedValue([{ code: 'DE', name: null }, { code: 'FR', name: null }]);
  backend.log.mockResolvedValue([]);
  backend.save.mockImplementation(async (input) => settings({ ...input, engines: useTranslationStore().settings!.engines }));
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  vi.clearAllMocks();
});

describe('Translation settings', () => {
  it('opens without asking DeepL anything', async () => {
    useTranslationStore().apply(settings({ engine: 'deepl', engines: [{ id: 'deepl', has_key: true, plan: 'pro', local: false }] }));
    wrapper = mount(TranslationSettings);
    await flushPromises();
    expect(backend.languages).toHaveBeenCalledWith('deepl', false);
    expect(backend.check).not.toHaveBeenCalled();

    await wrapper.get('[data-testid="translation-target"]').trigger('focus');
    await flushPromises();
    expect(backend.languages).toHaveBeenLastCalledWith('deepl', true);
    // The Pro plan's terms, not the free one's.
    expect(wrapper.find('[data-testid="translation-terms-pro"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="translation-terms-free"]').exists()).toBe(false);
  });

  it('withdrawing consent never switches translation on', async () => {
    useTranslationStore().apply(settings({
      consents: [{ engine: 'deepl', account_id: 'acc-1', granted_at: '2026-11-06T00:00:00Z' }],
    }));
    backend.consent.mockResolvedValue(settings());
    wrapper = mount(TranslationSettings);
    await flushPromises();
    const toggle = wrapper.get<HTMLInputElement>('[data-testid="translation-consent-acc-1"] input[role="switch"]');
    await toggle.setValue(false);
    await flushPromises();
    expect(backend.save).not.toHaveBeenCalled();
  });

  it('loads a log entry\'s sent text only when it is opened', async () => {
    backend.log.mockResolvedValue([{
      id: 7, created_at: '2026-11-06T10:00:00Z', feature: 'translation', provider: 'deepl', model: null,
      account_id: 'acc-1', message_id: 'm-1', units: 12, unit: 'characters', outcome: 'ok', has_sent_text: true,
    }]);
    backend.logText.mockResolvedValue('Hola a todos');
    wrapper = mount(TranslationSettings);
    await flushPromises();
    expect(backend.logText).not.toHaveBeenCalled();
    await wrapper.get('[data-testid="translation-log-entry"] button').trigger('click');
    await flushPromises();
    expect(backend.logText).toHaveBeenCalledWith(7);
    expect(wrapper.get('[data-testid="translation-log-text"]').text()).toBe('Hola a todos');
  });

  it('keeps a free key but no account on, and says why', async () => {
    useTranslationStore().apply(settings({
      engine: 'deepl',
      engines: [{ id: 'deepl', has_key: true, plan: 'free', local: false }],
      consents: [{ engine: 'deepl', account_id: 'acc-1', granted_at: '2026-11-06T00:00:00Z' }],
    }));
    backend.check.mockResolvedValue({ used: 10, limit: 500000, unit: 'characters' });
    wrapper = mount(TranslationSettings);
    await flushPromises();
    expect(wrapper.get('[data-testid="translation-free-refused"]').text())
      .toContain("DeepL API Free keys can't be used for mail");
    const toggle = wrapper.get<HTMLInputElement>('[data-testid="translation-consent-acc-1"] input[role="switch"]');
    expect(toggle.element.disabled).toBe(true);
    expect(toggle.element.checked).toBe(false);
    expect(wrapper.find('[data-testid="translation-terms-free"]').exists()).toBe(true);

    // Test still works, so the plan and allowance can be seen.
    await wrapper.get('[data-testid="translation-test"]').trigger('click');
    await flushPromises();
    expect(backend.check).toHaveBeenCalledWith('deepl');
    expect(wrapper.get('[data-testid="translation-usage"]').text()).toContain('500,000');
  });

  it('shows no refusal with a Pro key', async () => {
    useTranslationStore().apply(settings({ engine: 'deepl', engines: [{ id: 'deepl', has_key: true, plan: 'pro', local: false }] }));
    wrapper = mount(TranslationSettings);
    await flushPromises();
    expect(wrapper.find('[data-testid="translation-free-refused"]').exists()).toBe(false);
    const toggle = wrapper.get<HTMLInputElement>('[data-testid="translation-consent-acc-1"] input[role="switch"]');
    expect(toggle.element.disabled).toBe(false);
  });

  it('sends the key once, forgets it and switches DeepL on', async () => {
    backend.setKey.mockResolvedValue(settings({ engines: [{ id: 'deepl', has_key: true, plan: 'free', local: false }] }));
    wrapper = mount(TranslationSettings);
    await flushPromises();

    const input = wrapper.get<HTMLInputElement>('[data-testid="translation-key-input"]');
    expect(input.attributes('type')).toBe('password');
    await input.setValue(' abc:fx ');
    await wrapper.get('form').trigger('submit');
    await flushPromises();

    expect(backend.setKey).toHaveBeenCalledWith('deepl', 'abc:fx');
    expect(input.element.value).toBe('');
    expect(wrapper.text()).toContain('Key saved on this device (DeepL API Free)');
    expect(wrapper.text()).not.toContain('abc:fx');
    expect(backend.save).toHaveBeenCalledWith(expect.objectContaining({ engine: 'deepl' }));
  });

  it('shows the allowance left after a test', async () => {
    useTranslationStore().apply(settings({ engine: 'deepl', engines: [{ id: 'deepl', has_key: true, plan: 'free', local: false }] }));
    backend.check.mockResolvedValue({ used: 1200, limit: 500000, unit: 'characters' });
    wrapper = mount(TranslationSettings);
    await flushPromises();

    await wrapper.get('[data-testid="translation-test"]').trigger('click');
    await flushPromises();
    expect(wrapper.get('[data-testid="translation-usage"]').text()).toContain('1,200 of 500,000 characters used');
    expect(backend.languages).toHaveBeenLastCalledWith('deepl', true);
  });

  it('keeps every account off until it is switched on', async () => {
    backend.consent.mockResolvedValue(settings({
      engine: 'deepl',
      consents: [{ engine: 'deepl', account_id: 'acc-1', granted_at: '2026-11-06T00:00:00Z' }],
    }));
    wrapper = mount(TranslationSettings);
    await flushPromises();

    const toggle = wrapper.get<HTMLInputElement>('[data-testid="translation-consent-acc-1"] input[role="switch"]');
    expect(toggle.element.checked).toBe(false);
    await toggle.setValue(true);
    await flushPromises();
    expect(backend.consent).toHaveBeenCalledWith('deepl', 'acc-1', true);
  });
});
