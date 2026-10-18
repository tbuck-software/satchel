import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendRuntimeCapabilities,
  backendSupportsNativeCommands,
  type BackendRuntimeCapabilities,
} from '@/services/backend';
import { supportsDesktopBackgroundControls } from '@/utils/runtimeCapabilities';

export type SettingsSection =
  | 'general'
  | 'about'
  | 'swipes'
  | 'calendar'
  | 'accounts'
  | 'signatures'
  | 'aliases'
  | 'contacts'
  | 'smime'
  | 'pgp'
  | 'templates'
  | 'notifications'
  | 'sending'
  | 'data'
  | 'tray'
  | 'shortcuts'
  | 'rules'
  | 'plugins'
  | 'smart-folders'
  | 'provider-features'
  | 'vacation'
  | 'blocked'
  | 'images'
  | 'backup'
  | 'sources'
  | 'mobile-sync'
  | 'diagnostics';

export interface SettingsSectionGroup {
  title: string;
  items: Array<{ id: SettingsSection; label: string }>;
}

/**
 * What the shell reported about itself, or `null` while the answer is still
 * on its way.
 *
 * The window width used to stand in for this. It is a layout measure: a
 * narrowed desktop window hid working tray settings, and a tablet-sized phone
 * showed switches whose backend refuses them.
 */
const shellCapabilities = ref<BackendRuntimeCapabilities | null>(null);
const mobileSyncPlatform = ref<'android' | null>(null);
let capabilityLoaded = false;

/** Sections whose backing feature the shell has to actually have. */
function sectionIsSupported(
  section: SettingsSection,
  shell: BackendRuntimeCapabilities | null,
): boolean {
  switch (section) {
    // Both switches on that page call commands a phone answers with a refusal.
    case 'tray':
      return shell !== null && supportsDesktopBackgroundControls(shell);
    case 'shortcuts':
      return shell !== null && !shell.mobile;
    default:
      return true;
  }
}

type SettingsGroupId = 'app' | 'identity' | 'mail' | 'advanced';

interface SettingsSectionGroupDefinition {
  id: SettingsGroupId;
  items: SettingsSection[];
}

const BASE_GROUPS: SettingsSectionGroupDefinition[] = [
  {
    id: 'app',
    items: [
      'general',
      'swipes',
      'calendar',
      'notifications',
      'tray',
      'shortcuts',
      'about',
    ],
  },
  {
    id: 'identity',
    items: [
      'accounts',
      'contacts',
      'signatures',
      'aliases',
      'smime',
      'pgp',
    ],
  },
  {
    id: 'mail',
    items: [
      'sending',
      'data',
      'vacation',
      'blocked',
      'images',
      'smart-folders',
      'provider-features',
      'backup',
    ],
  },
  {
    id: 'advanced',
    items: [
      'sources',
      'templates',
      'rules',
      'plugins',
      'diagnostics',
    ],
  },
];

export const settingsSectionIcons: Record<SettingsSection, string> = {
  about: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7h.01"/>',
  general: '<path d="M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3M14 2v4M8 10v4M16 18v4"/>',
  swipes: '<path d="M5 12h14M8 9l-3 3 3 3M16 9l3 3-3 3"/><path d="M9.5 5.5c1.4-1.2 3.6-1.2 5 0M9.5 18.5c1.4 1.2 3.6 1.2 5 0"/>',
  calendar: '<rect width="18" height="18" x="3" y="4" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/>',
  notifications: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
  'mobile-sync': '<rect width="14" height="20" x="5" y="2" rx="2"/><path d="M12 18h.01M8.5 9.5a3.5 3.5 0 0 1 7 0"/>',
  tray: '<rect width="20" height="14" x="2" y="3" rx="2"/><path d="M8 21h8M12 17v4"/>',
  shortcuts: '<rect width="20" height="16" x="2" y="4" rx="2"/><path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M6 12h.01M10 12h.01M14 12h.01M18 12h.01M7 16h10"/>',
  accounts: '<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>',
  contacts: '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="10" r="3"/><path d="M7 20.66V19a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v1.66"/>',
  signatures: '<path d="M12 20h9M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
  aliases: '<circle cx="12" cy="12" r="4"/><path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8"/>',
  smime: '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1zM9 12l2 2 4-4"/>',
  pgp: '<path d="m21 2-9.6 9.6"/><circle cx="7.5" cy="15.5" r="5.5"/><path d="m15.5 7.5 3 3L22 7l-3-3"/>',
  sending: '<path d="M22 2 11 13M22 2 15 22l-4-9-9-4Z"/>',
  data: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5M3 12a9 3 0 0 0 18 0"/>',
  vacation: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/>',
  blocked: '<circle cx="12" cy="12" r="10"/><path d="m4.9 4.9 14.2 14.2"/>',
  images: '<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>',
  'smart-folders': '<path d="M10.7 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4.1"/><circle cx="17" cy="17" r="3"/><path d="m21 21-1.9-1.9"/>',
  'provider-features': '<path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"/>',
  backup: '<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8M10 12h4"/>',
  sources: '<path d="M4 11a9 9 0 0 1 9 9M4 4a16 16 0 0 1 16 16"/><circle cx="5" cy="19" r="1"/>',
  diagnostics: '<path d="M3 12h4l3 8 4-16 3 8h4"/>',
  templates: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7ZM14 2v4a2 2 0 0 0 2 2h4M10 9H8M16 13H8M16 17H8"/>',
  rules: '<path d="M22 3H2l8 9.46V19l4 2v-8.54Z"/>',
  plugins: '<path d="M12 2v5M12 17v5M4.93 4.93l3.54 3.54M15.53 15.53l3.54 3.54M2 12h5M17 12h5M4.93 19.07l3.54-3.54M15.53 8.47l3.54-3.54"/><circle cx="12" cy="12" r="3"/>',
};

export function resolveSettingsSection(value: unknown): SettingsSection {
  const requested = Array.isArray(value) ? value[0] : value;
  return BASE_GROUPS.flatMap((group) => group.items).some((item) => item === requested)
    || requested === 'mobile-sync'
    ? requested as SettingsSection
    : 'general';
}

async function loadShellCapabilities() {
  if (capabilityLoaded || !backendSupportsNativeCommands()) return;
  capabilityLoaded = true;
  try {
    const capabilities = await backendRuntimeCapabilities();
    shellCapabilities.value = capabilities;
    // Only Android has a foreground service whose health is worth a page.
    if (capabilities.platform === 'android') {
      mobileSyncPlatform.value = capabilities.platform;
    }
  } catch {
    // Older shells simply do not expose background sync settings.
  }
}

/** The sections this shell offers, grouped in sidebar order. Usable outside a component. */
export function availableSettingsSections(): Array<{ id: SettingsGroupId; items: SettingsSection[] }> {
  void loadShellCapabilities();
  return BASE_GROUPS.map((group) => {
    const items = [...group.items];
    if (group.id === 'app' && mobileSyncPlatform.value) {
      items.splice(3, 0, 'mobile-sync');
    }
    // Unanswered means hidden: a section that flashes and disappears is
    // worse than one that arrives a moment late.
    return { id: group.id, items: items.filter((item) => sectionIsSupported(item, shellCapabilities.value)) };
  }).filter((group) => group.items.length > 0);
}

export function useSettingsSections() {
  const { t } = useI18n();

  onMounted(() => loadShellCapabilities());

  const groups = computed<SettingsSectionGroup[]>(() =>
    availableSettingsSections().map((group) => ({
      title: t(`settings.sections.groups.${group.id}.title`),
      items: group.items.map((id) => ({ id, label: t(`settings.sections.${id}.label`) })),
    })),
  );

  const labels = computed(() => new Map(
    groups.value.flatMap((group) => group.items.map((item) => [item.id, item.label] as const)),
  ));

  return { groups, labels, mobileSyncPlatform };
}
