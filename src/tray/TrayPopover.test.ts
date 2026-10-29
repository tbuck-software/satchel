// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';

const { traySnapshot, openThread, openDestination, actOnThread } = vi.hoisted(() => ({
  traySnapshot: vi.fn(),
  openThread: vi.fn(),
  openDestination: vi.fn(),
  actOnThread: vi.fn(),
}));

const tauriWindow = vi.hoisted(() => ({
  focusHandler: undefined as ((event: { payload: boolean }) => void) | undefined,
  hide: vi.fn(),
  isVisible: vi.fn(),
  onFocusChanged: vi.fn(),
}));

vi.mock('@/services/backend', () => ({
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
  backendTraySnapshot: traySnapshot,
  backendOpenTrayThread: openThread,
  backendOpenTrayDestination: openDestination,
  backendActOnTrayThread: actOnThread,
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => tauriWindow,
}));

vi.mock('@/components/common/Avatar.vue', () => ({
  default: { template: '<span class="avatar-stub" />' },
}));

import TrayPopover from './TrayPopover.vue';

function mountPopover() {
  return mount(TrayPopover, { global: { plugins: [createPinia()] } });
}

describe('tray briefing', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriWindow.focusHandler = undefined;
    tauriWindow.isVisible.mockResolvedValue(true);
    tauriWindow.onFocusChanged.mockImplementation(async (handler) => {
      tauriWindow.focusHandler = handler;
      return () => {};
    });
    traySnapshot.mockResolvedValue({
      new_threads: [
        {
          id: 'thread-1',
          sender_name: 'Mara Hoffmann',
          sender_address: 'mara@example.com',
          subject: 'Sign-off for the next release',
          received_at: new Date().toISOString(),
          is_unread: true,
        },
      ],
      upcoming_events: [
        {
          title: 'Project meeting',
          starts_at: new Date(Date.now() + 20 * 60_000).toISOString(),
          ends_at: new Date(Date.now() + 80 * 60_000).toISOString(),
          all_day: false,
          location: 'Meet',
        },
        {
          title: 'Planning session',
          starts_at: new Date(Date.now() + 26 * 60 * 60_000).toISOString(),
          ends_at: new Date(Date.now() + 27 * 60 * 60_000).toISOString(),
          all_day: false,
          location: null,
        },
      ],
    });
  });

  it('labels all-day DATE values on their calendar day west of UTC', async () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(new Date(2026, 2, 28, 12).getTime());
    traySnapshot.mockResolvedValueOnce({ new_threads: [], upcoming_events: [{
      title: 'All-day holiday', starts_at: '2026-03-29', ends_at: '2026-03-30', all_day: true, location: null,
    }] });
    const wrapper = mountPopover();
    await flushPromises();
    expect(wrapper.text()).toContain('Tomorrow');
    expect(wrapper.text()).toContain('All day');
    wrapper.unmount();
    now.mockRestore();
  });

  it('loads only when the hidden popover is actually shown and refreshes on reopen', async () => {
    tauriWindow.isVisible.mockResolvedValueOnce(false);
    const wrapper = mountPopover();
    await flushPromises();

    expect(traySnapshot).not.toHaveBeenCalled();

    tauriWindow.focusHandler?.({ payload: true });
    tauriWindow.focusHandler?.({ payload: true });
    await flushPromises();
    expect(traySnapshot).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain('Mara Hoffmann');

    tauriWindow.focusHandler?.({ payload: false });
    tauriWindow.focusHandler?.({ payload: true });
    await flushPromises();
    expect(tauriWindow.hide).toHaveBeenCalledTimes(1);
    expect(traySnapshot).toHaveBeenCalledTimes(2);
  });

  it('shows the two workflow actions without adding read or reply controls', async () => {
    const wrapper = mountPopover();
    await flushPromises();

    expect(wrapper.text()).toContain('Just arrived');
    expect(wrapper.text()).toContain('Mara Hoffmann');
    expect(wrapper.text()).toContain('Sign-off for the next release');
    expect(wrapper.text()).toContain('Project meeting');
    expect(wrapper.text()).toContain('Planning session');
    expect(wrapper.find('[title="Mark as read"]').exists()).toBe(false);
    expect(wrapper.find('[title="Mark as done"]').exists()).toBe(true);
    expect(wrapper.find('[title="Wait for a reply"]').exists()).toBe(true);
    expect(wrapper.find('[title="Reply"]').exists()).toBe(false);
  });

  it('completes a mail without opening it and removes the handled row', async () => {
    actOnThread.mockResolvedValueOnce(undefined);
    const wrapper = mountPopover();
    await flushPromises();

    await wrapper.get('[title="Mark as done"]').trigger('click');
    await flushPromises();

    expect(actOnThread).toHaveBeenCalledWith('thread-1', 'done');
    expect(openThread).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain('All clear');
  });

  it('moves a mail to waiting from the same hover action group', async () => {
    actOnThread.mockResolvedValueOnce(undefined);
    const wrapper = mountPopover();
    await flushPromises();

    await wrapper.get('[title="Wait for a reply"]').trigger('click');
    await flushPromises();

    expect(actOnThread).toHaveBeenCalledWith('thread-1', 'waiting');
  });

  it('opens a selected mail in Satchel', async () => {
    const wrapper = mountPopover();
    await flushPromises();

    await wrapper.get('.mail-card__open').trigger('click');
    await flushPromises();

    expect(openThread).toHaveBeenCalledWith('thread-1');
  });

  it('opens calendar account settings from the header control', async () => {
    const wrapper = mountPopover();
    await flushPromises();

    expect(wrapper.text()).not.toContain('Overview');
    await wrapper.get('[title="Calendar settings"]').trigger('click');
    await flushPromises();

    expect(openDestination).toHaveBeenCalledWith('calendar_settings');
  });

  it('renders a calm empty state without inventing an unread count', async () => {
    traySnapshot.mockResolvedValueOnce({ new_threads: [], upcoming_events: [] });
    const wrapper = mountPopover();
    await flushPromises();

    expect(wrapper.text()).toContain('All clear');
    expect(wrapper.text()).toContain('No upcoming events found.');
    expect(wrapper.text()).not.toMatch(/\b\d+ unread\b/i);
  });
});
