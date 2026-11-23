import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  Activity,
  AtSign,
  Ban,
  Bell,
  Calendar,
  CircleHelp,
  Database,
  FileText,
  FlaskConical,
  FolderSearch,
  Keyboard,
  Languages,
  Lock,
  Mail,
  MoveHorizontal,
  Palmtree,
  Puzzle,
  Signature,
  SlidersHorizontal,
  SquarePen,
  Users,
  Workflow,
  type IconNode,
} from 'lucide';
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
let capabilityLoad: Promise<void> | null = null;

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

export const settingsSectionIcons: Record<SettingsSection, IconNode> = {
  about: CircleHelp,
  general: SlidersHorizontal,
  swipes: MoveHorizontal,
  calendar: Calendar,
  notifications: Bell,
  shortcuts: Keyboard,
  accounts: Mail,
  contacts: Users,
  signatures: Signature,
  aliases: AtSign,
  vacation: Palmtree,
  'smart-folders': FolderSearch,
  diagnostics: Activity,
  labs: FlaskConical,
  templates: FileText,
  rules: Workflow,
  plugins: Puzzle,
  composing: SquarePen,
  senders: Ban,
  encryption: Lock,
  translation: Languages,
  storage: Database,
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

function loadShellCapabilities() {
  if (shellCapabilities.value || !backendCommandsAvailable()) return;
  capabilityLoad ??= backendRuntimeCapabilities()
    .then((capabilities) => {
      shellCapabilities.value = capabilities;
      // Only Android has a foreground service whose health is worth a page.
      if (capabilities.platform === 'android') {
        mobileSyncPlatform.value = capabilities.platform;
      }
    })
    .catch(() => {
      // Keep capabilities unknown and allow the next caller to retry.
    })
    .finally(() => {
      capabilityLoad = null;
    });
  return capabilityLoad;
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

  const relayPlatform = computed(() => {
    const platform = shellCapabilities.value?.platform;
    return platform && ['macos', 'windows', 'linux', 'android'].includes(platform) ? platform : null;
  });
  return { groups, labels, mobileSyncPlatform, relayPlatform, trayAvailable };
}
