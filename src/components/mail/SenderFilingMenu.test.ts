// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, flushPromises } from '@vue/test-utils';
import SenderFilingMenu from './SenderFilingMenu.vue';

const { getPreferenceMock, pushMock } = vi.hoisted(() => ({
  getPreferenceMock: vi.fn(),
  pushMock: vi.fn(),
}));

vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendGetSenderPreference: getPreferenceMock,
}));

vi.mock('vue-router', () => ({ useRoute: () => ({}), useRouter: () => ({ push: pushMock }) }));

function mountMenu() {
  return mount(SenderFilingMenu, {
    props: { accountId: 'acc-1', email: 'Mike@Agency.example' },
  });
}

function rows(wrapper: ReturnType<typeof mountMenu>) {
  return wrapper.findAll('button').map((button) => ({
    label: button.text(),
    checked: button.attributes('aria-checked') === 'true',
  }));
}

beforeEach(() => {
  setActivePinia(createPinia());
  getPreferenceMock.mockReset();
  pushMock.mockReset();
  window.localStorage.clear();
});

describe('sender filing menu', () => {
  it('shows the current filing rather than an add/remove pair', async () => {
    getPreferenceMock.mockResolvedValue({
      sender_email: 'mike@agency.example',
      category: 'updates',
      is_priority: true,
    });

    const wrapper = mountMenu();
    await flushPromises();

    expect(getPreferenceMock).toHaveBeenCalledWith('mike@agency.example');
    expect(rows(wrapper)).toEqual([
      { label: 'Always prioritise', checked: true },
      { label: 'Personal', checked: false },
      { label: 'Notifications', checked: true },
      { label: 'Newsletter', checked: false },
      { label: 'Show all mail', checked: false },
    ]);
    expect(wrapper.text()).not.toContain('remove');
  });

  it('clears the priority when the checked row is picked again', async () => {
    getPreferenceMock.mockResolvedValue({
      sender_email: 'mike@agency.example',
      category: null,
      is_priority: true,
    });
    const wrapper = mountMenu();
    await flushPromises();

    const store = await import('@/stores/threads');
    const threads = store.useThreadsStore();
    const setPriority = vi.spyOn(threads, 'setSenderPriorityByEmail').mockResolvedValue();

    await wrapper.findAll('button')[0]!.trigger('click');
    await flushPromises();

    expect(setPriority).toHaveBeenCalledWith('acc-1', 'mike@agency.example', false);
    expect(wrapper.emitted('close')).toHaveLength(1);
  });

  it('falls back to the local bucket mirror when the command is unavailable', async () => {
    getPreferenceMock.mockRejectedValue(new Error('unknown command'));
    const settings = await import('@/stores/settings');
    settings.useSettingsStore().setSenderInboxBucket('mike@agency.example', 'newsletter');

    const wrapper = mountMenu();
    await flushPromises();

    expect(rows(wrapper).find((row) => row.label === 'Newsletter')?.checked).toBe(true);
  });

  it('keeps an explicit uncategorized backend preference over the local mirror', async () => {
    getPreferenceMock.mockResolvedValue({
      sender_email: 'mike@agency.example',
      category: null,
      is_priority: false,
    });
    const settings = await import('@/stores/settings');
    settings.useSettingsStore().setSenderInboxBucket('mike@agency.example', 'newsletter');

    const wrapper = mountMenu();
    await flushPromises();

    expect(rows(wrapper)).toEqual([
      { label: 'Always prioritise', checked: false },
      { label: 'Personal', checked: false },
      { label: 'Notifications', checked: false },
      { label: 'Newsletter', checked: false },
      { label: 'Show all mail', checked: false },
    ]);
  });

  it('ignores a preference response for a sender that is no longer shown', async () => {
    let resolveMike!: (preference: {
      sender_email: string;
      category: 'personal' | 'updates' | 'promotions' | 'social' | null;
      is_priority: boolean;
    }) => void;
    const mikePreference = new Promise((resolve) => {
      resolveMike = resolve;
    });
    getPreferenceMock.mockImplementation((email: string) => {
      if (email === 'mike@agency.example') return mikePreference;
      return Promise.resolve({ sender_email: email, category: 'personal', is_priority: false });
    });

    const wrapper = mountMenu();
    await wrapper.setProps({ email: 'jane@example.com' });
    await flushPromises();

    resolveMike({ sender_email: 'mike@agency.example', category: 'promotions', is_priority: true });
    await flushPromises();

    expect(rows(wrapper)).toEqual([
      { label: 'Always prioritise', checked: false },
      { label: 'Personal', checked: true },
      { label: 'Notifications', checked: false },
      { label: 'Newsletter', checked: false },
      { label: 'Show all mail', checked: false },
    ]);
  });

  it('ignores an old response after switching away from and back to a sender', async () => {
    let resolveFirstMike!: (preference: {
      sender_email: string;
      category: 'promotions';
      is_priority: boolean;
    }) => void;
    const firstMikePreference = new Promise((resolve) => {
      resolveFirstMike = resolve;
    });
    let mikeRequests = 0;
    getPreferenceMock.mockImplementation((email: string) => {
      if (email === 'mike@agency.example' && mikeRequests++ === 0) {
        return firstMikePreference;
      }
      return Promise.resolve({ sender_email: email, category: 'personal', is_priority: false });
    });

    const wrapper = mountMenu();
    await wrapper.setProps({ email: 'jane@example.com' });
    await wrapper.setProps({ email: 'mike@agency.example' });
    await flushPromises();

    resolveFirstMike({
      sender_email: 'mike@agency.example',
      category: 'promotions',
      is_priority: true,
    });
    await flushPromises();

    expect(rows(wrapper)).toEqual([
      { label: 'Always prioritise', checked: false },
      { label: 'Personal', checked: true },
      { label: 'Notifications', checked: false },
      { label: 'Newsletter', checked: false },
      { label: 'Show all mail', checked: false },
    ]);
  });

  it('shows all of the sender\'s mail in the list search, not the search page', async () => {
    getPreferenceMock.mockResolvedValue({ sender_email: 'mike@agency.example', category: null, is_priority: false });
    const wrapper = mountMenu();
    await flushPromises();

    await wrapper.findAll('button').at(-1)!.trigger('click');

    const { useListSearchStore } = await import('@/stores/listSearch');
    const listSearch = useListSearchStore();
    expect(listSearch.query).toBe('from:mike@agency.example');
    expect(listSearch.scope).toBe('all');
    expect(pushMock).toHaveBeenCalledWith({ name: 'inbox' });
    expect(wrapper.emitted('close')).toHaveLength(1);
  });
});
