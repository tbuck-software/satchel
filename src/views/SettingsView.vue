<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, defineAsyncComponent, onBeforeUnmount, ref } from 'vue';
import { useRouter } from 'vue-router';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import SearchInput from '@/components/ui/SearchInput.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useUiStore } from '@/stores/ui';
import { useAccountsStore } from '@/stores/accounts';
import ThemeSelector from '@/components/settings/ThemeSelector.vue';
import {
  resolveSettingsSection,
  settingsSectionIcons,
  useSettingsSections,
  type SettingsSection,
} from '@/components/settings/settingsSections';
import { useI18n } from 'vue-i18n';
import { registerSettingsFilterField } from '@/composables/settingsFilterFocus';
import { useSettingsFilter } from '@/composables/useSettingsFilter';
import { useSettingsLocation } from '@/composables/useSettingsLocation';

// Each section is its own chunk: opening Settings loads the section it shows,
// not all of them — the signature and template editors bring the rich-text
// editor along. Nothing renders while a chunk arrives; theme and language stay
// static because the general section is the default.
const AboutPane = defineAsyncComponent(() => import('@/components/settings/AboutPane.vue'));
const AccountSetup = defineAsyncComponent(() => import('@/components/settings/AccountSetup.vue'));
const AliasEditor = defineAsyncComponent(() => import('@/components/settings/AliasEditor.vue'));
const BackupExport = defineAsyncComponent(() => import('@/components/settings/BackupExport.vue'));
const BlockedSendersEditor = defineAsyncComponent(() => import('@/components/settings/BlockedSendersEditor.vue'));
const CalendarSettings = defineAsyncComponent(() => import('@/components/settings/CalendarSettings.vue'));
const ContactAvatarSettings = defineAsyncComponent(() => import('@/components/settings/ContactAvatarSettings.vue'));
const DiagnosticsPane = defineAsyncComponent(() => import('@/components/settings/DiagnosticsPane.vue'));
const InlineImageBackfill = defineAsyncComponent(() => import('@/components/settings/InlineImageBackfill.vue'));
const KeybindingEditor = defineAsyncComponent(() => import('@/components/settings/KeybindingEditor.vue'));
const MobileSyncSettings = defineAsyncComponent(() => import('@/components/settings/MobileSyncSettings.vue'));
const NotificationSettings = defineAsyncComponent(() => import('@/components/settings/NotificationSettings.vue'));
const PluginManager = defineAsyncComponent(() => import('@/components/settings/PluginManager.vue'));
const ProviderFeaturesEditor = defineAsyncComponent(() => import('@/components/settings/ProviderFeaturesEditor.vue'));
const RemoteImageAllowlist = defineAsyncComponent(() => import('@/components/settings/RemoteImageAllowlist.vue'));
const RuleBuilder = defineAsyncComponent(() => import('@/components/settings/RuleBuilder.vue'));
const SignaturesEditor = defineAsyncComponent(() => import('@/components/settings/SignaturesEditor.vue'));
const SmartFoldersEditor = defineAsyncComponent(() => import('@/components/settings/SmartFoldersEditor.vue'));
const SmimeManager = defineAsyncComponent(() => import('@/components/settings/SmimeManager.vue'));
const SourceManager = defineAsyncComponent(() => import('@/components/settings/SourceManager.vue'));
const SpamFilterSettings = defineAsyncComponent(() => import('@/components/settings/SpamFilterSettings.vue'));
const TemplatesEditor = defineAsyncComponent(() => import('@/components/settings/TemplatesEditor.vue'));
const SwipeSettings = defineAsyncComponent(() => import('@/components/settings/SwipeSettings.vue'));
const TrayPreferences = defineAsyncComponent(() => import('@/components/settings/TrayPreferences.vue'));
const VacationSettings = defineAsyncComponent(() => import('@/components/settings/VacationSettings.vue'));
const WebhooksEditor = defineAsyncComponent(() => import('@/components/settings/WebhooksEditor.vue'));

const router = useRouter();
const { t } = useI18n();
const ui = useUiStore();
const accounts = useAccountsStore();
const { groups, labels, mobileSyncPlatform, trayAvailable } = useSettingsSections();
const { query: filterQuery, filtering, results: filterResults } = useSettingsFilter();
const filterInput = ref<InstanceType<typeof SearchInput> | null>(null);
/** The phone overview filters like the desktop sidebar: one ranked list while typing. */
const overviewGroups = computed(() => (
  filtering.value
    ? [{ title: t('settings.search.results'), items: filterResults.value }]
    : groups.value.map((group) => ({
      title: group.title,
      items: group.items.map((item) => ({ ...item, hint: null as string | null })),
    }))
));
const { inDialog, section: locationSection, show: showSection } = useSettingsLocation();
const requestedSection = computed(() => {
  const value = locationSection.value;
  if (value === null) return null;
  // Folded sections (`smime`, `backup`, …) open the section that holds them now.
  const section = resolveSettingsSection(value);
  return labels.value.has(section) ? section : null;
});
const showOverview = computed(() => ui.isSinglePane && requestedSection.value === null);
const activeSection = computed(() => requestedSection.value ?? resolveSettingsSection(undefined));
const activeLabel = computed(() => t(`settings.sections.${activeSection.value}.label`));

function sectionLabel(section: SettingsSection): string {
  return t(`settings.sections.${section}.label`);
}

function sectionDescription(section: SettingsSection): string {
  return t(`settings.sections.${section}.description`);
}

const sectionTones: Record<SettingsSection, string> = {
  general: 'blue', accounts: 'blue', notifications: 'orange',
  composing: 'blue', signatures: 'orange', templates: 'blue', aliases: 'cyan', vacation: 'amber',
  rules: 'purple', 'smart-folders': 'purple', swipes: 'purple',
  senders: 'red', encryption: 'green',
  calendar: 'green', contacts: 'pink',
  shortcuts: 'purple', plugins: 'pink', storage: 'cyan', diagnostics: 'amber', about: 'blue',
};

/** Sections made of several editors get one page header of their own. */
const COMPOSITE: ReadonlySet<SettingsSection> = new Set(['senders', 'encryption', 'storage']);
const providerAccounts = computed(() =>
  accounts.emailAccounts.some((account) => account.provider === 'gmail' || account.provider === 'outlook'));

function openSection(section: SettingsSection) {
  showSection(section, ui.isSinglePane ? 'push' : 'replace');
}

function goBack() {
  router.back();
}

let unregisterFilter: (() => void) | null = null;
function bindOverviewFilter(element: unknown) {
  filterInput.value = element as InstanceType<typeof SearchInput> | null;
  unregisterFilter?.();
  unregisterFilter = element ? registerSettingsFilterField(() => filterInput.value?.focus()) : null;
}
onBeforeUnmount(() => unregisterFilter?.());
</script>

<template>
  <section
    class="settings-view"
    :class="{ 'settings-view--dialog': inDialog }"
  >
    <!-- In the dialog the section list names Settings and the section names
         itself; a bar saying "Settings" again would only push both down. -->
    <PaneHeader
      v-if="!inDialog"
      :class="{ 'settings-view__header': !ui.isSinglePane, 'settings-view__header--large': ui.isMobile && showOverview }"
      :title="showOverview ? $t('common.settings') : (ui.isSinglePane ? activeLabel : $t('common.settings'))"
      :subtitle="ui.isMobile ? undefined : showOverview ? $t('settings.overview.subtitle') : (ui.isSinglePane ? $t('common.settings') : undefined)"
    >
      <!-- The phone reaches the overview from its tab bar: it is a root, with
           nothing behind it to go back to. -->
      <template v-if="ui.isSinglePane && !(ui.isMobile && showOverview)" #leading>
        <IconButton :title="$t('common.back')" size="lg" @click="goBack">
          <AppIcon size="lg"><path d="m10.5 3-5 5 5 5" /></AppIcon>
        </IconButton>
      </template>
    </PaneHeader>

    <div v-if="showOverview" class="settings-overview">
      <div class="settings-overview__filter">
        <SearchInput
          :ref="bindOverviewFilter"
          v-model="filterQuery"
          data-settings-filter
          :placeholder="$t('settings.search.placeholder')"
          :label="$t('settings.search.label')"
        />
      </div>
      <p v-if="filtering && filterResults.length === 0" class="settings-overview__empty" role="status">
        {{ $t('settings.search.empty', { query: filterQuery.trim() }) }}
      </p>
      <section v-for="group in overviewGroups" :key="group.title" class="settings-overview__group">
        <h2>{{ group.title }}</h2>
        <div class="settings-overview__list">
          <CardButton
            v-for="section in group.items"
            :key="section.id"
            @click="openSection(section.id)"
          >
            <span
              class="settings-overview__icon"
              :class="`settings-overview__icon--${sectionTones[section.id]}`"
            >
              <AppIcon :grid="24"><g v-html="settingsSectionIcons[section.id]" /></AppIcon>
            </span>
            <span class="settings-overview__copy">
              <strong>{{ sectionLabel(section.id) }}</strong>
              <small>{{ section.hint ?? sectionDescription(section.id) }}</small>
            </span>
            <AppIcon class="settings-overview__chevron"><path d="m6 3 5 5-5 5" /></AppIcon>
          </CardButton>
        </div>
      </section>
    </div>

    <div v-else class="settings-view__scroll">
      <div class="settings-view__content">
        <header v-if="COMPOSITE.has(activeSection)" class="section-header">
          <h3 class="settings-page-title">{{ activeLabel }}</h3>
          <p>{{ sectionDescription(activeSection) }}</p>
        </header>
        <template v-if="activeSection === 'general'">
          <ThemeSelector />
          <section v-if="trayAvailable" class="settings-subsection"><TrayPreferences /></section>
        </template>
        <AboutPane v-else-if="activeSection === 'about'" />
        <SwipeSettings v-else-if="activeSection === 'swipes'" />
        <CalendarSettings
          v-else-if="activeSection === 'calendar'"
          @manage-accounts="openSection('accounts')"
        />
        <!-- What comes in and goes out: mail accounts, feeds, webhooks, and the
             labels Gmail and Outlook keep for an account. -->
        <template v-else-if="activeSection === 'accounts'">
          <AccountSetup />
          <section class="settings-subsection"><SourceManager /></section>
          <section class="settings-subsection"><WebhooksEditor /></section>
          <section v-if="providerAccounts" class="settings-subsection"><ProviderFeaturesEditor /></section>
        </template>
        <SignaturesEditor v-else-if="activeSection === 'signatures'" />
        <AliasEditor v-else-if="activeSection === 'aliases'" />
        <ContactAvatarSettings v-else-if="activeSection === 'contacts'" />
        <template v-else-if="activeSection === 'encryption'">
          <section class="settings-subsection"><SmimeManager /></section>
          <section class="settings-subsection"><PluginManager only="pgp" @open-integrations="openSection('plugins')" /></section>
        </template>
        <TemplatesEditor v-else-if="activeSection === 'templates'" />
        <template v-else-if="activeSection === 'notifications'">
          <NotificationSettings />
          <section v-if="mobileSyncPlatform" class="settings-subsection"><MobileSyncSettings /></section>
        </template>
        <NotificationSettings v-else-if="activeSection === 'composing'" section="sending" />
        <template v-else-if="activeSection === 'storage'">
          <section class="settings-subsection">
            <NotificationSettings section="data" />
            <InlineImageBackfill class="settings-subsection__part" />
          </section>
          <section class="settings-subsection settings-subsection--untitled"><BackupExport /></section>
        </template>
        <KeybindingEditor v-else-if="activeSection === 'shortcuts'" />
        <RuleBuilder v-else-if="activeSection === 'rules'" />
        <PluginManager v-else-if="activeSection === 'plugins'" only="plugins" />
        <SmartFoldersEditor v-else-if="activeSection === 'smart-folders'" />
        <VacationSettings v-else-if="activeSection === 'vacation'" />
        <template v-else-if="activeSection === 'senders'">
          <section class="settings-subsection"><SpamFilterSettings /></section>
          <section class="settings-subsection"><BlockedSendersEditor /></section>
          <section class="settings-subsection"><RemoteImageAllowlist /></section>
        </template>
        <DiagnosticsPane v-else-if="activeSection === 'diagnostics'" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.settings-view {
  /* The reading column every section sits in; the header follows it. */
  --settings-column: 760px;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  height: 100%;
  min-width: 0;
  min-height: 0;
  background: var(--bg-base);
}

.settings-view__scroll {
  min-height: 0;
  overflow-y: auto;
  /* A classic scrollbar would shift the centred column off the pane title
     above it; reserving the gutter on both sides keeps the two aligned. */
  scrollbar-gutter: stable both-edges;
  border-top: 1px solid var(--border-color-muted);
}

.settings-overview {
  min-height: 0;
  overflow-y: auto;
  padding: 8px 16px 48px;
  border-top: 1px solid var(--border-color-muted);
}

.settings-overview__group {
  margin-top: 22px;
}

.settings-overview__filter {
  margin-top: var(--space-3-5);
}

.settings-overview__empty {
  margin: 22px 4px 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.settings-overview__group h2 {
  margin: 0 4px 8px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-strong);
}

.settings-overview__list {
  overflow: hidden;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
}

.settings-overview__list button {
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr) 18px;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-height: 66px;
  padding: 9px 14px;
  border: 0;
  border-bottom: 1px solid var(--border-color-muted);
  background: transparent;
  color: var(--text-primary);
  font: inherit;
  text-align: left;
}

.settings-overview__list button:last-child { border-bottom: 0; }
.settings-overview__list button:active { background: var(--bg-active); }

.settings-overview__icon {
  display: grid;
  width: 32px;
  height: 32px;
  place-items: center;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--settings-tone) 14%, var(--bg-raised));
  color: var(--settings-tone);
}

.settings-overview__icon--blue { --settings-tone: var(--accent-primary); }
.settings-overview__icon--green { --settings-tone: var(--accent-green); }
.settings-overview__icon--amber { --settings-tone: var(--accent-amber); }
.settings-overview__icon--red { --settings-tone: var(--accent-red); }
.settings-overview__icon--purple { --settings-tone: var(--accent-purple); }
.settings-overview__icon--cyan { --settings-tone: var(--accent-cyan); }
.settings-overview__icon--pink { --settings-tone: var(--accent-pink); }
.settings-overview__icon--orange { --settings-tone: var(--accent-orange); }

.settings-overview__chevron { color: var(--text-tertiary); }

.settings-overview__copy {
  display: grid;
  min-width: 0;
  gap: var(--space-2xs);
}

.settings-overview__copy strong {
  color: var(--text-primary);
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
}

.settings-overview__copy small {
  overflow: hidden;
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-regular);
  line-height: 1.25;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.settings-view--dialog {
  grid-template-rows: minmax(0, 1fr);
}

.settings-view--dialog .settings-view__scroll {
  border-top: 0;
}

/* The pane title starts where the section does. Left at the pane's edge it
   sat a column's margin away from the cards below it on a wide window. */
.settings-view > .settings-view__header {
  padding-inline: max(
    var(--space-lg),
    calc((100% - var(--settings-column)) / 2 + var(--space-2xl))
  );
}

/* A readable column, centred in the pane: about 700px of card at any window
   width, so a row's copy and its control stay within one glance. Real data
   (a long address, a sync error, a backup path) breaks inside the column
   rather than pushing the page wider than the dialog. */
.settings-view__content {
  overflow-wrap: anywhere;
  container-type: inline-size;
  width: min(100%, var(--settings-column));
  margin-inline: auto;
  padding: var(--space-2xl) var(--space-2xl) calc(2 * var(--space-2xl));
}

/* Header rows (title beside its actions) wrap their actions under the title
   when there is no room, instead of overflowing. */
.settings-view__content :deep(header) {
  flex-wrap: wrap;
}

.settings-view__content :deep(header > *) {
  min-width: 0;
}

/* One title per section, whoever renders it: each editor brought its own
   size, and Rules, Backup and Signatures read as a form label instead. */
.settings-view__content :deep(.settings-page-title) {
  margin: 0;
  color: var(--text-primary);
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
  line-height: 1.2;
}

.settings-view__content :deep(.settings-page-title + p) {
  margin: var(--space-2xs) 0 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

/* The phone's Settings root is a root screen: a large title, as the mail
   lists have, rather than a nav-bar title over nothing. */
.settings-view__header--large :deep(.pane-header__title) {
  font-size: var(--font-size-2xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
}

/* A section made of several editors: each keeps its own title, one step
   down, and the parts sit apart. */
.settings-view__content > .settings-subsection,
.settings-view__content > .section-header {
  margin-bottom: var(--space-2xl);
}

.settings-view__content > .settings-subsection:last-child {
  margin-bottom: 0;
}

.settings-view__content > :not(.settings-subsection) + .settings-subsection {
  margin-top: var(--space-2xl);
}

.settings-subsection__part {
  margin-top: var(--space-2xl);
}

/* Its own groups (Export & import, Database backup) name it well enough under
   the page title; a third heading level only stacked up. */
.settings-view__content .settings-subsection.settings-subsection--untitled :deep(.settings-page-title),
.settings-view__content .settings-subsection.settings-subsection--untitled :deep(.settings-page-title + p) {
  display: none;
}

.settings-subsection :deep(.settings-page-title) {
  font-size: var(--font-size-lg);
  font-weight: var(--weight-semibold);
  letter-spacing: 0;
}

/* Native WebKit selects and number spinners paint in the document's scheme,
   not the app theme — a white menu in the dark theme otherwise. One rule for
   every section instead of one per editor. */
.settings-view__content :deep(select),
.settings-view__content :deep(input) {
  color-scheme: dark;
}

[data-theme='light'] .settings-view__content :deep(select),
[data-theme='light'] .settings-view__content :deep(input) {
  color-scheme: light;
}

/* Phones scroll with overlay indicators. A classic bar (or the gutter kept
   for one) pushed scrolling pages further in than short ones, and only on
   the right. */
@media (max-width: 767px) and (pointer: coarse) {
  .settings-view__scroll {
    scrollbar-gutter: auto;
    scrollbar-width: none;
  }

  .settings-view__scroll::-webkit-scrollbar {
    display: none;
  }
}

@media (max-width: 700px) {
  .settings-view__content {
    padding: var(--space-xl) var(--space-lg) var(--space-2xl);
  }
}

/* Phone: the nav bar already names the section; a second, larger heading
   under it said "General" twice. The section's description stays. */
@media (max-width: 767px) {
  .settings-view__content :deep(.section-header > h2),
  .settings-view__content :deep(.section-header > h3),
  .settings-view__content :deep(.settings-page-title) {
    display: none;
  }

  /* A part of a combined section names itself, also on the phone. */
  .settings-view__content .settings-subsection :deep(.settings-page-title),
  .settings-view__content .settings-subsection :deep(.section-header > h3) {
    display: block;
  }

  .settings-view__content :deep(.settings-page-title + p) {
    margin-top: 0;
  }

  .settings-view__content :deep(.section-header > p) {
    margin-top: 0;
  }
}
</style>
