// @vitest-environment jsdom

import { defineComponent } from 'vue';
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  route: { query: {} as Record<string, unknown> },
  push: vi.fn(),
  openSettings: vi.fn(),
  addressBook: {
    contacts: [] as Array<{
      email: string;
      display_name?: string | null;
      notes?: string | null;
    }>,
    loaded: true,
    loading: false,
    error: null as unknown,
    load: vi.fn(),
  },
  threads: {
    allContactSummaries: [] as Array<{
      email: string;
      name: string;
      threadCount: number;
      previewText: string;
    }>,
    openContact: vi.fn(),
  },
  settings: {
    pinnedContacts: [] as string[],
  },
}));

vi.mock('@/stores/ui', () => ({ useUiStore: () => ({ openSettings: harness.openSettings }) }));

vi.mock('vue-router', () => ({
  useRoute: () => harness.route,
  useRouter: () => ({ push: harness.push }),
}));

vi.mock('@/stores/contacts', () => ({
  useContactsStore: () => harness.addressBook,
}));

vi.mock('@/stores/threads', () => ({
  useThreadsStore: () => harness.threads,
}));

vi.mock('@/stores/settings', () => ({
  useSettingsStore: () => harness.settings,
}));

import ContactsView from './ContactsView.vue';

const PaneHeaderStub = defineComponent({
  name: 'PaneHeader',
  props: {
    title: { type: String, required: true },
    subtitle: { type: String, required: true },
    count: { type: Number, required: true },
  },
  template: '<header data-test="pane-header" :data-title="title" :data-subtitle="subtitle" :data-count="count"><slot name="actions" /></header>',
});

const AvatarStub = defineComponent({
  name: 'Avatar',
  props: {
    name: { type: String, required: true },
    email: { type: String, required: false },
  },
  template: '<span data-test="avatar" :data-name="name" :data-email="email" />',
});

function render() {
  return mount(ContactsView, {
    global: {
      stubs: {
        PaneHeader: PaneHeaderStub,
        AddContactDialog: true,
        Avatar: AvatarStub,
      },
    },
  });
}

describe('ContactsView', () => {
  beforeEach(() => {
    harness.openSettings.mockReset();
    harness.route.query = {};
    harness.push.mockReset().mockResolvedValue(undefined);
    harness.addressBook.contacts = [];
    harness.addressBook.loaded = true;
    harness.addressBook.loading = false;
    harness.addressBook.error = null;
    harness.addressBook.load.mockReset();
    harness.threads.allContactSummaries = [];
    harness.threads.openContact.mockReset();
    harness.settings.pinnedContacts = [];
  });

  it('offers address-book setup from the header and empty state, and opens contact creation', async () => {
    const wrapper = render();
    expect(wrapper.get('.empty h3').text()).toBe('No contacts found yet.');
    expect(wrapper.get('.empty p').text()).toContain('mail correspondents and address-book sync');
    await wrapper.get('.empty button').trigger('click');
    expect(harness.openSettings).toHaveBeenLastCalledWith('contacts');
    const actions = wrapper.findAll('header button');
    await actions[0]!.trigger('click');
    expect(harness.openSettings).toHaveBeenCalledTimes(2);
    await actions[1]!.trigger('click');
    expect(wrapper.findComponent({ name: 'AddContactDialog' }).exists()).toBe(true);
  });

  it('normalizes and deduplicates email keys while preserving address-book names and notes', () => {
    harness.addressBook.contacts = [
      {
        email: ' Alice@Example.com ',
        display_name: ' Alice ',
        notes: '  Project colleague  ',
      },
    ];
    harness.threads.allContactSummaries = [
      {
        email: 'alice@example.com',
        name: 'Frequent Alice',
        threadCount: 2,
        previewText: 'Latest conversation',
      },
      {
        email: 'bob@example.com',
        name: 'Bob',
        threadCount: 1,
        previewText: 'A note from Bob',
      },
    ];
    harness.settings.pinnedContacts = ['carol@example.com'];

    const wrapper = render();
    const rows = wrapper.findAll('.contact-list button');

    expect(rows).toHaveLength(3);
    expect(rows.map((row) => row.text())).toEqual([
      expect.stringContaining('Alice'),
      expect.stringContaining('Bob'),
      expect.stringContaining('carol@example.com'),
    ]);
    expect(rows[0]?.text()).toContain('Project colleague');
    expect(rows[0]?.text()).not.toContain('Latest conversation');
    expect(rows[0]?.text()).toContain('2 conversations');
    expect(wrapper.find('[data-test="pane-header"]').attributes('data-count')).toBe('3');
  });

  it('uses the route section to select only frequent contacts', () => {
    harness.route.query = { section: 'frequent' };
    harness.addressBook.contacts = [
      { email: 'address-book@example.com', display_name: 'Address Book' },
    ];
    harness.threads.allContactSummaries = [
      {
        email: 'frequent@example.com',
        name: 'Frequent Person',
        threadCount: 3,
        previewText: 'Frequent preview',
      },
    ];

    const wrapper = render();

    expect(wrapper.find('[data-test="pane-header"]').attributes('data-title')).toBe(
      'Frequently contacted',
    );
    expect(wrapper.findAll('.contact-list button')).toHaveLength(1);
    expect(wrapper.text()).toContain('Frequent Person');
    expect(wrapper.text()).not.toContain('Address Book');
  });

  it('supports pinned sections, including pinned addresses absent from other sources', () => {
    harness.route.query = { section: ['pinned', 'all'] };
    harness.addressBook.contacts = [
      { email: 'pinned@example.com', display_name: 'Pinned Person' },
      { email: 'not-pinned@example.com', display_name: 'Other Person' },
    ];
    harness.settings.pinnedContacts = ['pinned@example.com', 'orphan@example.com'];

    const wrapper = render();

    expect(wrapper.find('[data-test="pane-header"]').attributes('data-title')).toBe('Pinned');
    expect(wrapper.findAll('.contact-list button')).toHaveLength(2);
    expect(wrapper.text()).toContain('Pinned Person');
    expect(wrapper.text()).toContain('orphan@example.com');
    expect(wrapper.text()).not.toContain('Other Person');
  });

  /**
   * An address book that failed to load and one that was never imported look
   * identical unless the view says which of the two it is.
   */
  it('shows the failure instead of the empty state when the query failed', async () => {
    harness.addressBook.error = new Error('database is locked');

    const wrapper = render();

    expect(wrapper.find('.empty').exists()).toBe(false);
    const state = wrapper.get('[role="alert"]');
    expect(state.text()).toContain('The address book could not be loaded.');

    await state.get('button').trigger('click');
    expect(harness.addressBook.load).toHaveBeenCalledOnce();
  });

  it('groups and filters contacts by name, email and notes, and distinguishes no matches', async () => {
    harness.addressBook.contacts = [
      { email: 'zoe@example.com', display_name: 'Zoë' },
      { email: 'alice@example.com', display_name: 'Alice', notes: 'Design team' },
      { email: 'amy@example.com', display_name: 'Amy' },
    ];
    const wrapper = render();
    expect(wrapper.findAll('.section-heading__label').map((heading) => heading.text())).toEqual(['A', 'Z']);
    expect(wrapper.findAll('.contact-row strong').map((name) => name.text())).toEqual(['Alice', 'Amy', 'Zoë']);
    for (const query of ['ALICE', 'alice@', 'design']) {
      await wrapper.get('input[type="search"]').setValue(query);
      expect(wrapper.findAll('.contact-row')).toHaveLength(1);
      expect(wrapper.get('.contact-row').text()).toContain('Alice');
    }
    await wrapper.get('input').setValue('missing');
    expect(wrapper.get('.empty h3').text()).toBe('No matching contacts');
    expect(wrapper.get('[data-test="pane-header"]').attributes('data-count')).toBe('0');
    await wrapper.get('input').setValue('');
    expect(wrapper.findAll('.contact-row')).toHaveLength(3);
  });

  it('omits redundant source tags in address book and frequent views', () => {
    harness.addressBook.contacts = [{ email: 'alice@example.com', display_name: 'Alice' }];
    expect(render().get('.source-tag').text()).toBe('Address book');
    harness.route.query = { section: 'address-book' };
    expect(render().find('.source-tag').exists()).toBe(false);
    harness.route.query = { section: 'frequent' };
    harness.threads.allContactSummaries = [{ email: 'alice@example.com', name: 'Alice', threadCount: 2, previewText: '' }];
    expect(render().find('.source-tag').exists()).toBe(false);
  });

  it('opens the contact filter before navigating back to the inbox', async () => {
    harness.threads.allContactSummaries = [
      {
        email: 'person@example.com',
        name: 'Person',
        threadCount: 1,
        previewText: 'A conversation',
      },
    ];

    const wrapper = render();
    await wrapper.get('.contact-list button').trigger('click');

    expect(harness.threads.openContact).toHaveBeenCalledOnce();
    expect(harness.threads.openContact).toHaveBeenCalledWith('person@example.com');
    expect(harness.push).toHaveBeenCalledOnce();
    expect(harness.push).toHaveBeenCalledWith({ name: 'inbox' });
    expect(harness.threads.openContact.mock.invocationCallOrder[0]!).toBeLessThan(
      harness.push.mock.invocationCallOrder[0]!,
    );
  });
});
