// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { afterEach, expect, it } from 'vitest';
import { applyLocalePreference } from '@/i18n/runtime';
import SmimeStatusCard from './SmimeStatusCard.vue';

afterEach(() => applyLocalePreference('en', false));

it.each([
  ['en', 'The S/MIME message has too many nested layers.'],
  ['de', 'Die S/MIME-Nachricht enthält zu viele verschachtelte Ebenen.'],
] as const)('renders the decoded failure cause in %s', async (locale, expected) => {
  await applyLocalePreference(locale, false);
  const wrapper = mount(SmimeStatusCard, { props: {
    state: 'error', errorDetail: JSON.stringify({ key: 'backend.errors.smimeNestingLimit', params: {} }),
  } });
  expect(wrapper.text()).toContain(expected);
  expect(wrapper.text()).not.toContain('S/MIME nesting limit exceeded');
  wrapper.unmount();
});

it('never renders a raw parser diagnostic as user-facing copy', () => {
  const wrapper = mount(SmimeStatusCard, { props: {
    state: 'signature_invalid', errorDetail: 'OpenSSL: synthetic technical diagnostic',
  } });
  expect(wrapper.text()).not.toContain('synthetic technical diagnostic');
  wrapper.unmount();
});
