// @vitest-environment jsdom

import { computed, defineComponent, reactive, ref } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { enableAutoUnmount, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SettingsSection, SettingsSectionGroup } from '@/components/settings/settingsSections';

type SettingsRoute = {
  query: Record<string, unknown>;
};

const harness = vi.hoisted(() => ({
  route: undefined as SettingsRoute | undefined,
  router: {
    back: vi.fn(),
    push: vi.fn(),
    replace: vi.fn(),
  },
  ui: undefined as { isSinglePane: boolean } | undefined,
  groups: undefined as { value: SettingsSectionGroup[] } | undefined,
  labels: undefined as { value: Map<SettingsSection, string> } | undefined,
  mobileSyncPlatform: undefined as { value: 'android' | null } | undefined,
}));

vi.mock('vue-router', () => ({
  useRoute: () => harness.route,
  useRouter: () => harness.router,
}));

vi.mock('@/stores/ui', () => ({
  useUiStore: () => harness.ui,
}));

vi.mock('@/components/settings/settingsSections', () => ({
  settingsSectionIcons: {
    general: '',
    accounts: '',
    backup: '',
    diagnostics: '',
  },
  resolveSettingsSection: (value: unknown): SettingsSection => {
    const requested = Array.isArray(value) ? value[0] : value;
    return requested === 'accounts' || requested === 'backup' || requested === 'diagnostics'
      ? requested
      : 'general';
  },
  useSettingsSections: () => ({
    groups: harness.groups,
    labels: harness.labels,
    mobileSyncPlatform: harness.mobileSyncPlatform,
  }),
}));

import SettingsView from './SettingsView.vue';

enableAutoUnmount(afterEach);

const PaneHeaderStub = defineComponent({
  props: {
    title: { type: String, required: true },
    subtitle: { type: String, default: '' },
    noMenu: { type: Boolean, default: false },
  },
  template: `
    <header data-test="pane-header" :data-title="title" :data-subtitle="subtitle" :data-no-menu="noMenu">
      <div data-test="leading"><slot name="leading" /></div>
      <slot />
    </header>
  `,
});

const IconButtonStub = defineComponent({
  inheritAttrs: false,
  props: {
    title: { type: String, default: undefined },
  },
  emits: ['click'],
  template: '<button class="icon-button" :title="title" @click="$emit(\'click\', $event)"><slot /></button>',
});

function paneStub(name: string) {
  return defineComponent({
    name: `${name}Stub`,
    template: `<div data-test="settings-pane" data-pane="${name}" />`,
  });
}

const paneStubs = {
  AccountSetup: paneStub('accounts'),
  AliasEditor: paneStub('aliases'),
  BackupExport: paneStub('backup'),
  BlockedSendersEditor: paneStub('blocked'),
  CalendarSettings: paneStub('calendar'),
  ContactAvatarSettings: paneStub('contacts'),
  DiagnosticsPane: paneStub('diagnostics'),
  InlineImageBackfill: paneStub('inline-image-backfill'),
  KeybindingEditor: paneStub('shortcuts'),
  LanguageSelector: paneStub('language'),
  MobileSyncSettings: paneStub('mobile-sync'),
  NotificationSettings: paneStub('notifications'),
  PluginManager: paneStub('plugins'),
  ProviderFeaturesEditor: paneStub('provider-features'),
  RemoteImageAllowlist: paneStub('images'),
  RuleBuilder: paneStub('rules'),
  SignaturesEditor: paneStub('signatures'),
  SmartFoldersEditor: paneStub('smart-folders'),
  SmimeManager: paneStub('smime'),
  SourceManager: paneStub('sources'),
  SwipeSettings: paneStub('swipes'),
  TemplatesEditor: paneStub('templates'),
  ThemeSelector: paneStub('theme'),
  TrayPreferences: paneStub('tray'),
  VacationSettings: paneStub('vacation'),
};

function mountView(options: { isSinglePane?: boolean; section?: unknown } = {}) {
  const { isSinglePane = true, section } = options;
  harness.route = reactive({
    query: section === undefined ? {} : { section },
  });
  harness.ui = reactive({ isSinglePane });
  harness.router.back.mockReset();
  harness.router.push.mockReset().mockResolvedValue(undefined);
  harness.router.replace.mockReset().mockResolvedValue(undefined);

  return mount(SettingsView, {
    global: {
      stubs: {
        IconButton: IconButtonStub,
        PaneHeader: PaneHeaderStub,
        ...paneStubs,
      },
    },
  });
}

beforeEach(() => {
  setActivePinia(createPinia());
  const groups: SettingsSectionGroup[] = [
    {
      title: 'Settings',
      items: [
        { id: 'general', label: 'General' },
        { id: 'accounts', label: 'Accounts' },
        { id: 'backup', label: 'Backup' },
        { id: 'diagnostics', label: 'Diagnostics' },
      ],
    },
  ];
  harness.groups = computed(() => groups);
  harness.labels = computed(() => new Map(groups[0]!.items.map((item) => [item.id, item.label])));
  harness.mobileSyncPlatform = ref(null);
});

describe('SettingsView', () => {
  it('shows the overview in single-pane mode and switches to the detail when mode changes', async () => {
    const wrapper = mountView();

    expect(wrapper.find('.settings-overview').exists()).toBe(true);
    expect(wrapper.find('.settings-view__scroll').exists()).toBe(false);
    expect(wrapper.find('[data-test="settings-pane"]').exists()).toBe(false);

    harness.ui!.isSinglePane = false;
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.settings-overview').exists()).toBe(false);
    expect(wrapper.get('[data-test="settings-pane"]').attributes('data-pane')).toBe('theme');
  });

  it('pushes the selected section from the single-pane overview', async () => {
    const wrapper = mountView();
    const accounts = wrapper.findAll('.settings-overview__list button')
      .find((button) => button.text().includes('Accounts'));

    expect(accounts).toBeDefined();
    await accounts!.trigger('click');

    expect(harness.router.push).toHaveBeenCalledOnce();
    expect(harness.router.push).toHaveBeenCalledWith({
      name: 'settings',
      query: { section: 'accounts' },
    });
    expect(harness.router.replace).not.toHaveBeenCalled();
  });

  it('uses a valid route section to select the corresponding pane', async () => {
    const wrapper = mountView({ section: ['backup', 'general'] });

    expect(wrapper.find('.settings-overview').exists()).toBe(false);
    expect(wrapper.get('[data-test="settings-pane"]').attributes('data-pane')).toBe('backup');
    expect(wrapper.get('[data-test="pane-header"]').attributes('data-title')).toBe('Backup & export');

    harness.route!.query = { section: 'diagnostics' };
    await wrapper.vm.$nextTick();

    expect(wrapper.get('[data-test="settings-pane"]').attributes('data-pane')).toBe('diagnostics');
  });

  it('falls back to the general pane outside single-pane mode for an unknown section', () => {
    const wrapper = mountView({ isSinglePane: false, section: 'not-a-section' });

    expect(wrapper.find('.settings-overview').exists()).toBe(false);
    expect(wrapper.get('[data-test="settings-pane"]').attributes('data-pane')).toBe('theme');
  });

  it('goes back from a single-pane detail view', async () => {
    const wrapper = mountView({ section: 'diagnostics' });

    await wrapper.get('[data-test="leading"] .icon-button').trigger('click');

    expect(harness.router.back).toHaveBeenCalledOnce();
    expect(harness.router.push).not.toHaveBeenCalled();
    expect(harness.router.replace).not.toHaveBeenCalled();
  });
});
