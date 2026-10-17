// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LOCALE_STORAGE_KEY } from '@/i18n/locale';
import { applyLocalePreference } from '@/i18n/runtime';
import LanguageSelector from './LanguageSelector.vue';

describe('LanguageSelector', () => {
  beforeEach(() => {
    window.localStorage.removeItem(LOCALE_STORAGE_KEY);
    applyLocalePreference('en', false);
    setActivePinia(createPinia());
  });

  afterEach(() => {
    applyLocalePreference('en', false);
    window.localStorage.removeItem(LOCALE_STORAGE_KEY);
  });

  it('loads and persists a locale override', async () => {
    const wrapper = mount(LanguageSelector);

    await wrapper.get('button:nth-of-type(3)').trigger('click');

    await vi.waitFor(() => expect(wrapper.get('.setting-label').text()).toBe('Sprache'));
    expect(document.documentElement.lang).toBe('de');
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe('de');

    await wrapper.get('button:nth-of-type(2)').trigger('click');

    expect(wrapper.get('.setting-label').text()).toBe('Language');
    expect(document.documentElement.lang).toBe('en');
    expect(window.localStorage.getItem(LOCALE_STORAGE_KEY)).toBe('en');
  });
});
