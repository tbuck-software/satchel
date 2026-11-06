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
}));
vi.mock('@/services/backend/translation', async (importOriginal) => ({
  ...await importOriginal<typeof import('@/services/backend/translation')>(),
  backendSetTranslationApiKey: backend.setKey,
  backendCheckTranslationEngine: backend.check,
  backendSaveTranslationSettings: backend.save,
  backendSetTranslationConsent: backend.consent,
  backendListTranslationLanguages: backend.languages,
  backendListOutboundRequests: backend.log,
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
