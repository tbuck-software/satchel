// @vitest-environment jsdom
import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { applyLocalePreference } from '@/i18n/runtime';
import { useWorkflowSyncStore } from '@/stores/workflowSync';
import AccountWorkflowSyncNote from './AccountWorkflowSyncNote.vue';

describe('the local-only note on the account card', () => {
  beforeEach(() => setActivePinia(createPinia()));
  afterEach(async () => { await applyLocalePreference('en', false); });

  it('says nothing for an account whose server stores Done and Snooze', () => {
    useWorkflowSyncStore().localOnly = ['acc-other'];
    const wrapper = mount(AccountWorkflowSyncNote, { props: { accountId: 'acc-work' } });
    expect(wrapper.find('[data-testid="workflow-sync-local-only"]').exists()).toBe(false);
  });

  it('explains that Done and Snooze stay on this device, in both languages', async () => {
    useWorkflowSyncStore().localOnly = ['acc-work'];
    const wrapper = mount(AccountWorkflowSyncNote, { props: { accountId: 'acc-work' } });
    const note = wrapper.get('[data-testid="workflow-sync-local-only"]');
    expect(note.text()).toContain('Done and Snooze stay on this device');
    expect(note.text()).toContain('still moved to Archive');

    await applyLocalePreference('de', false);
    await wrapper.vm.$nextTick();
    expect(note.text()).toContain('Erledigt und Wiedervorlage bleiben auf diesem Gerät');
  });
});
