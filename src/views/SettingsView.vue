<script setup lang="ts">
import CardButton from '@/components/ui/CardButton.vue';
import { computed, defineAsyncComponent, onBeforeUnmount, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { useUiStore } from '@/stores/ui';
import ThemeSelector from '@/components/settings/ThemeSelector.vue';
import LanguageSelector from '@/components/settings/LanguageSelector.vue';
import {
  resolveSettingsSection,
  settingsSectionIcons,
  useSettingsSections,
  type SettingsSection,
} from '@/components/settings/settingsSections';
import { useI18n } from 'vue-i18n';
import { registerSettingsFilterField } from '@/composables/settingsFilterFocus';
import { useSettingsFilter } from '@/composables/useSettingsFilter';

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
const TemplatesEditor = defineAsyncComponent(() => import('@/components/settings/TemplatesEditor.vue'));
const SwipeSettings = defineAsyncComponent(() => import('@/components/settings/SwipeSettings.vue'));
const TrayPreferences = defineAsyncComponent(() => import('@/components/settings/TrayPreferences.vue'));
const VacationSettings = defineAsyncComponent(() => import('@/components/settings/VacationSettings.vue'));

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const ui = useUiStore();
const { groups, labels, mobileSyncPlatform } = useSettingsSections();
const { query: filterQuery, filtering, results: filterResults } = useSettingsFilter();
const filterInput = ref<HTMLInputElement | null>(null);
/** The phone overview filters like the desktop sidebar: one ranked list while typing. */
const overviewGroups = computed(() => (
  filtering.value
    ? [{ title: t('settings.search.results'), items: filterResults.value }]
    : groups.value.map((group) => ({
      title: group.title,
      items: group.items.map((item) => ({ ...item, hint: null as string | null })),
    }))
));
const requestedSection = computed(() => {
  const value = Array.isArray(route.query.section) ? route.query.section[0] : route.query.section;
  return typeof value === 'string' && labels.value.has(value as SettingsSection)
    ? value as SettingsSection
    : null;
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
  about: 'blue',
  general: 'blue', swipes: 'purple', calendar: 'green', notifications: 'orange',
  'mobile-sync': 'cyan', tray: 'blue', shortcuts: 'purple', accounts: 'blue',
  contacts: 'pink', signatures: 'orange', aliases: 'cyan', smime: 'green', pgp: 'purple',
  sending: 'blue', data: 'cyan', vacation: 'amber', blocked: 'red', images: 'green',
  'smart-folders': 'purple', 'provider-features': 'cyan', backup: 'orange', sources: 'orange',
  templates: 'blue', rules: 'purple', plugins: 'pink',
  diagnostics: 'amber',
};

function openSection(section: SettingsSection) {
  const navigation = { name: 'settings' as const, query: { section } };
  if (ui.isSinglePane) {
    void router.push(navigation);
  } else {
    void router.replace(navigation);
  }
}

function goBack() {
  router.back();
}

let unregisterFilter: (() => void) | null = null;
function bindOverviewFilter(element: unknown) {
  filterInput.value = element as HTMLInputElement | null;
  unregisterFilter?.();
  unregisterFilter = element ? registerSettingsFilterField(() => filterInput.value?.focus()) : null;
}
onBeforeUnmount(() => unregisterFilter?.());
</script>

<template>
  <section class="settings-view">
    <PaneHeader
      :title="showOverview ? $t('common.settings') : (ui.isSinglePane ? activeLabel : $t('common.settings'))"
      :subtitle="showOverview ? $t('settings.overview.subtitle') : (ui.isSinglePane ? $t('common.settings') : activeLabel)"
      :no-menu="ui.isSinglePane"
    >
      <template v-if="ui.isSinglePane" #leading>
        <IconButton :title="$t('common.back')" size="lg" @click="goBack">
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="m10.5 3-5 5 5 5" />
          </svg>
        </IconButton>
      </template>
    </PaneHeader>

    <div v-if="showOverview" class="settings-overview">
      <input
        :ref="bindOverviewFilter"
        v-model="filterQuery"
        class="settings-overview__filter"
        type="search"
        data-settings-filter
        :placeholder="$t('settings.search.placeholder')"
        :aria-label="$t('settings.search.label')"
      />
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
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" v-html="settingsSectionIcons[section.id]" />
            </span>
            <span class="settings-overview__copy">
              <strong>{{ sectionLabel(section.id) }}</strong>
              <small>{{ section.hint ?? sectionDescription(section.id) }}</small>
            </span>
            <svg class="settings-overview__chevron" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="m6 3 5 5-5 5" />
            </svg>
          </CardButton>
        </div>
      </section>
    </div>

    <div v-else class="settings-view__scroll">
      <div
        class="settings-view__content"
        :class="{ 'settings-view__content--wide': activeSection === 'shortcuts' }"
      >
        <template v-if="activeSection === 'general'">
          <ThemeSelector />
          <div class="language-setting">
            <LanguageSelector />
          </div>
        </template>
        <AboutPane v-else-if="activeSection === 'about'" />
        <SwipeSettings v-else-if="activeSection === 'swipes'" />
        <CalendarSettings
          v-else-if="activeSection === 'calendar'"
          @manage-accounts="openSection('accounts')"
        />
        <AccountSetup v-else-if="activeSection === 'accounts'" />
        <SignaturesEditor v-else-if="activeSection === 'signatures'" />
        <AliasEditor v-else-if="activeSection === 'aliases'" />
        <ContactAvatarSettings v-else-if="activeSection === 'contacts'" />
        <SmimeManager v-else-if="activeSection === 'smime'" />
        <PluginManager v-else-if="activeSection === 'pgp'" only="pgp" />
        <TemplatesEditor v-else-if="activeSection === 'templates'" />
        <NotificationSettings v-else-if="activeSection === 'notifications'" />
        <MobileSyncSettings v-else-if="activeSection === 'mobile-sync' && mobileSyncPlatform" />
        <NotificationSettings v-else-if="activeSection === 'sending'" section="sending" />
        <template v-else-if="activeSection === 'data'">
          <NotificationSettings section="data" />
          <InlineImageBackfill />
        </template>
        <TrayPreferences v-else-if="activeSection === 'tray'" />
        <KeybindingEditor v-else-if="activeSection === 'shortcuts'" />
        <RuleBuilder v-else-if="activeSection === 'rules'" />
        <SourceManager v-else-if="activeSection === 'sources'" />
        <PluginManager v-else-if="activeSection === 'plugins'" only="plugins" />
        <SmartFoldersEditor v-else-if="activeSection === 'smart-folders'" />
        <ProviderFeaturesEditor v-else-if="activeSection === 'provider-features'" />
        <VacationSettings v-else-if="activeSection === 'vacation'" />
        <BlockedSendersEditor v-else-if="activeSection === 'blocked'" />
        <RemoteImageAllowlist v-else-if="activeSection === 'images'" />
        <BackupExport v-else-if="activeSection === 'backup'" />
        <DiagnosticsPane v-else-if="activeSection === 'diagnostics'" />
      </div>
    </div>
  </section>
</template>

<style scoped>
.settings-view {
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
  width: 100%;
  height: 44px;
  margin-top: 14px;
  padding: 0 var(--space-md);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-md);
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

.settings-overview__icon svg { width: 17px; height: 17px; }
.settings-overview__chevron { width: 16px; color: var(--text-tertiary); }

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

.settings-view__content {
  container-type: inline-size;
  width: min(100%, 980px);
  margin-inline: auto;
  padding: var(--space-2xl) var(--space-2xl) calc(2 * var(--space-2xl));
}

.settings-view__content--wide {
  width: min(100%, 980px);
}

.language-setting {
  margin-top: var(--space-lg);
  overflow: hidden;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
}

@media (max-width: 700px) {
  .settings-view__content {
    padding: var(--space-xl) var(--space-lg) var(--space-2xl);
  }
}
</style>
