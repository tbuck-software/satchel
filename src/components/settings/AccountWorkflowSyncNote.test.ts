// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { applyLocalePreference } from '@/i18n/runtime';
import { useWorkflowSyncStore } from '@/stores/workflowSync';
import AccountWorkflowSyncNote from './AccountWorkflowSyncNote.vue';

function note(reason: string) {
  useWorkflowSyncStore().localOnly = [{ account_id: 'acc-work', reason }];
  return mount(AccountWorkflowSyncNote, { props: { accountId: 'acc-work' } });
}

describe('the local-only note on the account card', () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(async () => { await applyLocalePreference('en', false); });

  it('says nothing for an account whose server stores Done and Snooze', () => {
    useWorkflowSyncStore().localOnly = [{ account_id: 'acc-other', reason: 'no_custom_keywords' }];
    const wrapper = mount(AccountWorkflowSyncNote, { props: { accountId: 'acc-work' } });
    expect(wrapper.find('[data-testid="workflow-sync-local-only"]').exists()).toBe(false);
  });

  it('explains a server without custom keywords, in both languages', async () => {
    const wrapper = note('no_custom_keywords');
    const banner = wrapper.get('[data-testid="workflow-sync-local-only"]');
    expect(banner.text()).toContain('Done and Snooze stay on this device');
    expect(banner.text()).toContain('does not store custom keywords');

    await applyLocalePreference('de', false);
    await wrapper.vm.$nextTick();
    expect(banner.text()).toContain('Erledigt und Wiedervorlage bleiben auf diesem Gerät');
  });

  it('says a refused write is tried again, not that keywords are unsupported', () => {
    const text = note('store_refused').text();
    expect(text).toContain('refused to store');
    expect(text).toContain('tries again');
    expect(text).not.toContain('does not store custom keywords');
  });

  it('points Gmail over IMAP to Google sign-in', () => {
    const text = note('gmail_imap').text();
    expect(text).toContain('Gmail over IMAP');
    expect(text).not.toContain('does not store custom keywords');
  });
});
