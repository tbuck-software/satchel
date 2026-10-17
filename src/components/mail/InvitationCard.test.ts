// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useUiStore } from '@/stores/ui';

const harness = vi.hoisted(() => ({ respond: vi.fn() }));

vi.mock('@/stores/invites', () => ({
  useInvitesStore: () => ({ respond: harness.respond }),
}));

vi.mock('@/utils/openExternalUrl', () => ({
  openExternalUrl: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('@/services/backend', () => ({
  backendOpenExternalUrl: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
}));

import { openExternalUrl } from '@/utils/openExternalUrl';
import InvitationCard from './InvitationCard.vue';

const invite = {
  id: 'invite-1',
  message_id: 'message-1',
  account_id: 'account-1',
  uid: 'series@example.com',
  organizer_email: 'alice@example.com',
  organizer_name: 'Alice',
  summary: 'Monatlicher Termin',
  description: null as string | null,
  location: null as string | null,
  starts_at: '2026-09-24T07:00:00Z',
  ends_at: '2026-09-24T07:30:00Z',
  all_day: 0,
  rsvp_status: 'needs-action',
  method: 'REQUEST',
  cancelled: 0,
  raw_ics: 'BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n',
  sequence: 0,
  recurring: 1,
  recurrence_unbounded: 1,
  recurrence_id: null,
  created_at: '2026-08-21T06:38:11Z',
  updated_at: '2026-08-21T06:38:11Z',
};

function render(overrides: Partial<typeof invite> = {}) {
  return mount(InvitationCard, { props: { invite: { ...invite, ...overrides } } });
}

function action(wrapper: ReturnType<typeof render>, label: string) {
  const button = wrapper.findAll('.actions button').find((candidate) =>
    candidate.text().includes(label),
  );
  if (!button) throw new Error(`missing action ${label}`);
  return button;
}

describe('recurring invitation safety', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    harness.respond.mockResolvedValue(undefined);
    vi.spyOn(window, 'confirm').mockReturnValue(true);
  });

  it('identifies an unbounded series and requires explicit confirmation', async () => {
    vi.mocked(window.confirm).mockReturnValue(false);
    const wrapper = render();

    expect(wrapper.text()).toContain('This series has no end date');
    expect(action(wrapper, 'Accept series').exists()).toBe(true);

    await action(wrapper, 'Accept series').trigger('click');

    expect(window.confirm).toHaveBeenCalledWith(expect.stringContaining('whole series'));
    expect(harness.respond).not.toHaveBeenCalled();
  });

  it('does not warn for a bounded series and prevents duplicate submission', async () => {
    let finish!: () => void;
    harness.respond.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    const wrapper = render({ recurrence_unbounded: 0 });
    const button = action(wrapper, 'Accept series');

    await button.trigger('click');
    await wrapper.vm.$nextTick();
    await button.trigger('click');

    expect(window.confirm).not.toHaveBeenCalled();
    expect(harness.respond).toHaveBeenCalledTimes(1);
    expect(button.attributes()).toHaveProperty('disabled');

    finish();
    await flushPromises();
  });

  it('surfaces persistence failures and lets the user retry', async () => {
    harness.respond.mockRejectedValueOnce(new Error('Outbox unavailable'));
    const wrapper = render({ recurrence_unbounded: 0 });

    await action(wrapper, 'Accept series').trigger('click');
    await flushPromises();

    expect(wrapper.get('[role="alert"]').text()).toContain('Outbox unavailable');
    expect(action(wrapper, 'Accept series').attributes()).not.toHaveProperty('disabled');
  });
});

// #125: the location opened unassessed and the notes asked through `window.confirm`.
describe('invitation links', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
  });

  it('asks before opening a risky meeting location', async () => {
    const wrapper = render({ location: 'https://203.0.113.7/meet' });

    await wrapper.get('.meta__link').trigger('click');
    await flushPromises();

    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('IP address instead of a domain');
    wrapper.unmount();
  });

  it('opens a plain meeting location directly in a desktop layout', async () => {
    useUiStore().isSinglePane = false;
    const wrapper = render({ location: 'https://meet.example.com/room' });

    await wrapper.get('.meta__link').trigger('click');
    await flushPromises();

    expect(openExternalUrl).toHaveBeenCalledWith('https://meet.example.com/room');
    wrapper.unmount();
  });

  it('asks before opening an untrusted plain meeting location in a single-pane layout', async () => {
    useUiStore().isSinglePane = true;
    const wrapper = render({ location: 'https://meet.example.com/room' });

    await wrapper.get('.meta__link').trigger('click');
    await flushPromises();

    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(wrapper.findComponent({ name: 'ExternalLinkDialog' }).props('assessment')).toMatchObject({
      hostname: 'meet.example.com',
    });
    wrapper.unmount();
  });

  it('confirms risky note links in the app dialog, not in window.confirm', async () => {
    const wrapper = render({ description: 'Dial in: https://example.test:8443/join' });

    await wrapper.get('.notes a').trigger('click');
    await flushPromises();

    expect(window.confirm).not.toHaveBeenCalled();
    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('unusual network port');
    wrapper.unmount();
  });
});
