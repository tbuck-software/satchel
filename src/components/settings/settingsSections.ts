import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendCommandsAvailable,
  backendRuntimeCapabilities,
  type BackendRuntimeCapabilities,
} from '@/services/backend';
import { supportsDesktopBackgroundControls } from '@/utils/runtimeCapabilities';

export type SettingsSection =
  | 'general'
  | 'accounts'
  | 'notifications'
  | 'composing'
  | 'signatures'
  | 'templates'
  | 'aliases'
  | 'vacation'
  | 'rules'
  | 'smart-folders'
  | 'swipes'
  | 'senders'
  | 'encryption'
  | 'translation'
  | 'calendar'
  | 'contacts'
  | 'shortcuts'
  | 'plugins'
  | 'storage'
  | 'diagnostics'
  | 'labs'
  | 'about';

/**
 * Sections that were folded into another one, and where they live now.
 * Links from toasts, banners, the tray and older builds still name them.
 */
const MOVED_SECTIONS: Record<string, SettingsSection> = {
  sending: 'composing',
  smime: 'encryption',
  pgp: 'encryption',
  blocked: 'senders',
  images: 'senders',
  spam: 'senders',
  data: 'storage',
  backup: 'storage',
  sources: 'accounts',
  'provider-features': 'accounts',
  tray: 'general',
  'mobile-sync': 'notifications',
};

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
    case 'shortcuts':
      return shell !== null && !shell.mobile;
    // Plugins run in the desktop runtime only; a phone has none to list.
    case 'plugins':
      return shell !== null && shell.plugins;
    default:
      return true;
  }
}

type SettingsGroupId = 'essentials' | 'writing' | 'organising' | 'privacy' | 'people' | 'app';

interface SettingsSectionGroupDefinition {
  id: SettingsGroupId;
  items: SettingsSection[];
}

/**
 * Grouped by what someone came to do, most visited first. Grouped by kind
 * (App, Identity, Mail, Advanced) Accounts hid under "Identity" and Rules under
 * "Advanced", and the list needed a scroll before Sending.
 */
const BASE_GROUPS: SettingsSectionGroupDefinition[] = [
  { id: 'essentials', items: ['general', 'accounts', 'notifications'] },
  { id: 'writing', items: ['composing', 'signatures', 'templates', 'aliases', 'vacation'] },
  { id: 'organising', items: ['rules', 'smart-folders', 'swipes'] },
  { id: 'privacy', items: ['senders', 'encryption', 'translation'] },
  { id: 'people', items: ['calendar', 'contacts'] },
  { id: 'app', items: ['shortcuts', 'plugins', 'storage', 'diagnostics', 'labs', 'about'] },
];

export const settingsSectionIcons: Record<SettingsSection, string> = {
  about: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7h.01"/>',
  general: '<path d="M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3M14 2v4M8 10v4M16 18v4"/>',
  swipes: '<path d="M5 12h14M8 9l-3 3 3 3M16 9l3 3-3 3"/><path d="M9.5 5.5c1.4-1.2 3.6-1.2 5 0M9.5 18.5c1.4 1.2 3.6 1.2 5 0"/>',
  calendar: '<rect width="18" height="18" x="3" y="4" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/>',
  notifications: '<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.94 1.94 0 0 0 3.4 0"/>',
  shortcuts: '<rect width="20" height="16" x="2" y="4" rx="2"/><path d="M6 8h.01M10 8h.01M14 8h.01M18 8h.01M6 12h.01M10 12h.01M14 12h.01M18 12h.01M7 16h10"/>',
  accounts: '<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>',
  contacts: '<circle cx="12" cy="12" r="10"/><circle cx="12" cy="10" r="3"/><path d="M7 20.66V19a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v1.66"/>',
  signatures: '<path d="M12 20h9M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
  aliases: '<circle cx="12" cy="12" r="4"/><path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8"/>',
  vacation: '<path d="M9 17 4 12l5-5"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>',
  'smart-folders': '<path d="M10.7 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v4.1"/><circle cx="17" cy="17" r="3"/><path d="m21 21-1.9-1.9"/>',
  diagnostics: '<path d="M3 12h4l3 8 4-16 3 8h4"/>',
  labs: '<path d="M9 3h6M10 3v6.5L4.6 18.3A1.8 1.8 0 0 0 6.1 21h11.8a1.8 1.8 0 0 0 1.5-2.7L14 9.5V3M7 15h10"/>',
  templates: '<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7ZM14 2v4a2 2 0 0 0 2 2h4M10 9H8M16 13H8M16 17H8"/>',
  rules: '<path d="M22 3H2l8 9.46V19l4 2v-8.54Z"/>',
  plugins: '<path d="M12 2v5M12 17v5M4.93 4.93l3.54 3.54M15.53 15.53l3.54 3.54M2 12h5M17 12h5M4.93 19.07l3.54-3.54M15.53 8.47l3.54-3.54"/><circle cx="12" cy="12" r="3"/>',
  composing: '<path d="M22 2 11 13M22 2 15 22l-4-9-9-4Z"/>',
  senders: '<circle cx="12" cy="12" r="10"/><path d="m4.9 4.9 14.2 14.2"/>',
  encryption: '<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1zM9 12l2 2 4-4"/>',
  translation: '<path d="m5 8 6 6M4 14l6-6 2-3M2 5h12M7 2h1M22 22l-5-10-5 10M14 18h6"/>',
  storage: '<ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5M3 12a9 3 0 0 0 18 0"/>',
};

/** A requested section, following moved ones; unknown requests open General. */
export function resolveSettingsSection(value: unknown): SettingsSection {
  const requested = Array.isArray(value) ? value[0] : value;
  if (typeof requested !== 'string') return 'general';
  if (requested in MOVED_SECTIONS) return MOVED_SECTIONS[requested]!;
  return BASE_GROUPS.some((group) => group.items.includes(requested as SettingsSection))
    ? requested as SettingsSection
    : 'general';
}

async function loadShellCapabilities() {
  if (capabilityLoaded || !backendCommandsAvailable()) return;
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

  /** Tray and autostart switches: a desktop shell that has them. Part of General. */
  const trayAvailable = computed(() =>
    shellCapabilities.value !== null && supportsDesktopBackgroundControls(shellCapabilities.value));

  return { groups, labels, mobileSyncPlatform, trayAvailable };
}
