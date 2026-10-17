// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';

const permission = vi.hoisted(() => ({ ensureNotificationPermission: vi.fn() }));
vi.mock('@/composables/useNotifications', () => permission);

const backend = vi.hoisted(() => ({
  backendGetNotificationPreferences: vi.fn(),
  backendSetNotificationPreferences: vi.fn(),
  backendSetBackgroundSync: vi.fn(),
  backendCommandsAvailable: vi.fn(() => true),
  backendSupportsNativeCommands: vi.fn(() => true),
}));
vi.mock('@/services/backend', () => backend);

import NotificationSettings from './NotificationSettings.vue';

const PREFS = {
  enabled: false,
  scope: 'inbox',
  sound: true,
  undo_send_seconds: 5,
  trash_auto_delete_days: 30,
  send_read_receipts: false,
};

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  backend.backendGetNotificationPreferences.mockResolvedValue({ ...PREFS });
  // Echoes its input, as the real command does — it returns what it stored.
  // A fixed reply made the switch snap back to the old value after saving.
  backend.backendSetNotificationPreferences.mockImplementation(async (next) => ({ ...next }));
  backend.backendSetBackgroundSync.mockResolvedValue(undefined);
});

async function mountSettings() {
  const wrapper = mount(NotificationSettings);
  await vi.waitFor(() => expect(backend.backendGetNotificationPreferences).toHaveBeenCalled());
  await wrapper.vm.$nextTick();
  return wrapper;
}

// ToggleSwitch is a clipped checkbox behind a painted track (UI-20), so the
// state is the input's own property rather than an aria attribute.
const toggle = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('input[role="switch"]')[0]!;

const isOn = (wrapper: ReturnType<typeof mount>) =>
  (toggle(wrapper).element as HTMLInputElement).checked;

async function flip(wrapper: ReturnType<typeof mount>) {
  await toggle(wrapper).setValue(!isOn(wrapper));
  await flushPromises();
}

describe('turning notifications on', () => {
  /**
   * Android 13 and newer post nothing until the user has been asked, and refuse
   * silently. The preference used to read "an" while nothing ever arrived,
   * because nothing in the app ever requested the permission.
   */
  it('asks the system before claiming to be on', async () => {
    permission.ensureNotificationPermission.mockResolvedValue(true);
    const wrapper = await mountSettings();

    await flip(wrapper);

    expect(permission.ensureNotificationPermission).toHaveBeenCalledOnce();
    expect(isOn(wrapper)).toBe(true);
  });

  it('stays off and says why when the system refuses', async () => {
    permission.ensureNotificationPermission.mockResolvedValue(false);
    const wrapper = await mountSettings();

    await flip(wrapper);

    expect(isOn(wrapper)).toBe(false);
    expect(wrapper.text()).toContain('system settings');
  });

  it('does not ask again just to turn them off', async () => {
    permission.ensureNotificationPermission.mockResolvedValue(true);
    const wrapper = await mountSettings();

    await flip(wrapper);
    permission.ensureNotificationPermission.mockClear();

    await flip(wrapper);

    expect(permission.ensureNotificationPermission).not.toHaveBeenCalled();
    expect(isOn(wrapper)).toBe(false);
  });
});

/**
 * The switch used to change nothing until a Speichern button elsewhere on the
 * page was pressed. It had visibly moved, so it read as applied, and a user who
 * walked away got no notifications and no hint why.
 */
describe('the switch itself', () => {
  it('persists without waiting for the save button', async () => {
    permission.ensureNotificationPermission.mockResolvedValue(true);
    const wrapper = await mountSettings();

    await flip(wrapper);

    expect(backend.backendSetNotificationPreferences).toHaveBeenCalledWith(
      expect.objectContaining({ enabled: true }),
    );
  });

  /** Off means the process no longer has to be held open. */
  it('takes the background connection with it', async () => {
    permission.ensureNotificationPermission.mockResolvedValue(true);
    const wrapper = await mountSettings();

    await flip(wrapper);
    expect(backend.backendSetBackgroundSync).toHaveBeenLastCalledWith(true);

    await flip(wrapper);
    expect(backend.backendSetBackgroundSync).toHaveBeenLastCalledWith(false);
  });
});
