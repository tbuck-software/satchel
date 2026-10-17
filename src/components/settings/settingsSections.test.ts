// @vitest-environment jsdom
import { defineComponent } from 'vue';
import { mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackendRuntimeCapabilities } from '@/services/backend';

const backend = vi.hoisted(() => ({ backendRuntimeCapabilities: vi.fn() }));
vi.mock('@/services/backend', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/services/backend')>()),
  backendCommandsAvailable: () => true,
  backendSupportsNativeCommands: () => true,
  ...backend,
}));

const uiStore = vi.hoisted(() => ({ isMobile: false }));
vi.mock('@/stores/ui', () => ({ useUiStore: () => uiStore }));

const DESKTOP: BackendRuntimeCapabilities = {
  platform: 'macos',
  mobile: false,
  tray: true,
  autostart: true,
  single_instance: true,
  plugins: true,
  native_crypto: true,
  background_sync: false,
  contacts: true,
  file_associations: true,
  deep_links: false,
  push: false,
};

const PHONE: BackendRuntimeCapabilities = {
  ...DESKTOP,
  platform: 'android',
  mobile: true,
  tray: false,
  autostart: false,
  single_instance: false,
  plugins: false,
  native_crypto: false,
  background_sync: true,
  file_associations: false,
  deep_links: true,
  push: true,
};

/**
 * `useSettingsSections` caches the shell answer in module state, so every case
 * needs a fresh module instance.
 */
async function sectionsFor(capabilities: BackendRuntimeCapabilities): Promise<string[]> {
  vi.resetModules();
  backend.backendRuntimeCapabilities.mockResolvedValue(capabilities);
  const { useSettingsSections } = await import('./settingsSections');
  let sections: string[] = [];
  const host = defineComponent({
    setup() {
      const { groups } = useSettingsSections();
      return () => {
        sections = groups.value.flatMap((group) => group.items.map((item) => item.id));
        return null;
      };
    },
  });
  const wrapper = mount(host);
  await vi.waitFor(() => expect(backend.backendRuntimeCapabilities).toHaveBeenCalled());
  await Promise.resolve();
  await wrapper.vm.$nextTick();
  return sections;
}

beforeEach(() => {
  vi.clearAllMocks();
  uiStore.isMobile = false;
});

describe('settings section visibility', () => {
  it('shows tray and shortcuts on a desktop shell, however narrow the window is', async () => {
    // The window width is a layout measure; dragging it narrow changes nothing.
    uiStore.isMobile = true;

    const sections = await sectionsFor(DESKTOP);

    expect(sections).toContain('tray');
    expect(sections).toContain('shortcuts');
  });

  it('hides them on a mobile shell, however wide the tablet is', async () => {
    uiStore.isMobile = false;

    const sections = await sectionsFor(PHONE);

    expect(sections).not.toContain('tray');
    expect(sections).not.toContain('shortcuts');
    // Android is the one platform whose background service earns a page.
    expect(sections).toContain('mobile-sync');
  });

  /** Replaces a regular expression over this file's source (ARCH-13). */
  it('offers the background-sync page on Android only', async () => {
    expect(await sectionsFor({ ...PHONE, platform: 'ios' })).not.toContain('mobile-sync');
    expect(await sectionsFor(DESKTOP)).not.toContain('mobile-sync');
  });

  it('follows tray and autostart separately from the mobile flag', async () => {
    const trayless = await sectionsFor({ ...DESKTOP, tray: false, autostart: true });
    expect(trayless).toContain('tray');

    const neither = await sectionsFor({ ...DESKTOP, tray: false, autostart: false });
    expect(neither).not.toContain('tray');
    // Not a phone, so the keyboard page stays.
    expect(neither).toContain('shortcuts');
  });

  it('shows neither section before the shell has answered', async () => {
    vi.resetModules();
    let resolveCapabilities: (value: BackendRuntimeCapabilities) => void = () => {};
    backend.backendRuntimeCapabilities.mockReturnValue(
      new Promise<BackendRuntimeCapabilities>((resolve) => {
        resolveCapabilities = resolve;
      }),
    );
    const { useSettingsSections } = await import('./settingsSections');
    let sections: string[] = [];
    const host = defineComponent({
      setup() {
        const { groups } = useSettingsSections();
        return () => {
          sections = groups.value.flatMap((group) => group.items.map((item) => item.id));
          return null;
        };
      },
    });
    mount(host);
    await Promise.resolve();

    expect(sections).not.toContain('tray');
    expect(sections).not.toContain('shortcuts');
    resolveCapabilities(DESKTOP);
  });
});
