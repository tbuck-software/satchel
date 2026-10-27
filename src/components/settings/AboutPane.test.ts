// @vitest-environment jsdom
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AboutPane from './AboutPane.vue';
import { useAppUpdateStore } from '@/stores/appUpdate';

const backend = vi.hoisted(() => ({
  backendAppBuildInfo: vi.fn(),
  backendOpenExternalUrl: vi.fn(),
  backendCheckAppUpdate: vi.fn(),
  backendGetAppUpdatePreferences: vi.fn(),
  backendSetAppUpdateChannel: vi.fn(),
}));
vi.mock('@/services/backend', async importOriginal => ({
  ...await importOriginal<typeof import('@/services/backend')>(),
  backendCommandsAvailable: () => true,
  ...backend,
}));

beforeEach(() => {
  vi.resetAllMocks();
  setActivePinia(createPinia());
  backend.backendAppBuildInfo.mockResolvedValue({ version: '0.1.1', commit: '48b6d7c', os: 'macos', arch: 'aarch64', debug: false });
  backend.backendCheckAppUpdate.mockResolvedValue({ channel: 'stable', version: null });
  backend.backendGetAppUpdatePreferences.mockResolvedValue({ channel: 'stable', supported: true });
  backend.backendSetAppUpdateChannel.mockResolvedValue(undefined);
  backend.backendOpenExternalUrl.mockResolvedValue(undefined);
});

describe('About Satchel', () => {
  it.each(['0.1.14', '0.1.15-nightly.27'])(
    'shows installed %s separately from the followed nightly channel',
    async version => {
      backend.backendAppBuildInfo.mockResolvedValue({ version, commit: '48b6d7c', os: 'macos', arch: 'aarch64', debug: false });
      backend.backendGetAppUpdatePreferences.mockResolvedValue({ channel: 'nightly', supported: true });
      const wrapper = mount(AboutPane);
      await flushPromises();
      const build = wrapper.get('[data-testid="about-build"]');
      expect(build.text()).toContain('Installed version');
      expect(build.text()).toContain(version);
      expect(wrapper.text()).toContain('Following nightly and stable releases, whichever is newer.');
      expect(wrapper.get<HTMLInputElement>('input[role="switch"]').element.checked).toBe(true);
    },
  );

  it('shows when the installed app follows only stable releases', async () => {
    const wrapper = mount(AboutPane);
    await flushPromises();
    expect(wrapper.text()).toContain('Following stable releases only.');
  });

  it('switches the same installed app to nightly updates', async () => {
    const wrapper = mount(AboutPane);
    await flushPromises();
    const toggle = wrapper.get<HTMLInputElement>('input[role="switch"]');
    expect(toggle.element.checked).toBe(false);
    await toggle.setValue(true);
    await flushPromises();
    expect(backend.backendSetAppUpdateChannel).toHaveBeenCalledWith('nightly');
    expect(toggle.element.checked).toBe(true);
    expect(backend.backendCheckAppUpdate).toHaveBeenCalledOnce();
  });

  it('restores the switch when the channel cannot be saved', async () => {
    backend.backendSetAppUpdateChannel.mockRejectedValueOnce(new Error('cannot save'));
    const wrapper = mount(AboutPane);
    await flushPromises();
    const toggle = wrapper.get<HTMLInputElement>('input[role="switch"]');
    await toggle.setValue(true);
    await flushPromises();
    expect(toggle.element.checked).toBe(false);
    expect(wrapper.text()).toContain('cannot save');
    expect(backend.backendCheckAppUpdate).not.toHaveBeenCalled();
  });
  it('shows the running binary version and lets the user check for updates', async () => {
    const wrapper = mount(AboutPane);
    await flushPromises();
    expect(wrapper.get('[data-testid="about-build"]').text()).toContain('0.1.1');
    expect(wrapper.get('[data-testid="about-build"]').text()).toContain('48b6d7c');
    await wrapper.findAll('button').find(button => button.text() === 'Check for updates')!.trigger('click');
    await flushPromises();
    expect(backend.backendCheckAppUpdate).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain('up to date');
  });

  it('keeps About visible when the version command fails and can retry', async () => {
    backend.backendAppBuildInfo.mockRejectedValueOnce(new Error('Unavailable'));
    const wrapper = mount(AboutPane);
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain('Version information could not be loaded');
    expect(wrapper.text()).toContain('MIT license');
    await wrapper.findAll('button').find(button => button.text() === 'Try again')!.trigger('click');
    await flushPromises();
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(wrapper.get('[data-testid="about-build"]').text()).toContain('0.1.1');
  });

  it('offers download and restart through the shared update state', async () => {
    const download = vi.fn().mockResolvedValue(undefined);
    const restart = vi.fn().mockResolvedValue(undefined);
    const updates = useAppUpdateStore();
    updates.offerUpdate({ version: '0.1.2', download, restart });
    const wrapper = mount(AboutPane);
    await flushPromises();
    await wrapper.findAll('button').find(button => button.classes().includes('app-button--primary'))!.trigger('click');
    await flushPromises();
    expect(download).toHaveBeenCalledOnce();
    await wrapper.findAll('button').find(button => button.classes().includes('app-button--primary'))!.trigger('click');
    await flushPromises();
    expect(restart).toHaveBeenCalledOnce();
  });

  it('opens the public source and license through the backend', async () => {
    const wrapper = mount(AboutPane);
    await flushPromises();
    for (const label of ['Source code', 'MIT license', 'Third-party licenses']) {
      await wrapper.findAll('button').find(button => button.text() === label)!.trigger('click');
    }
    expect(backend.backendOpenExternalUrl.mock.calls.map(call => call[0])).toEqual([
      'https://github.com/tbuck-software/satchel',
      'https://github.com/tbuck-software/satchel/blob/main/LICENSE',
      'https://github.com/tbuck-software/satchel/blob/main/THIRD_PARTY_NOTICES.md',
    ]);
  });
});
