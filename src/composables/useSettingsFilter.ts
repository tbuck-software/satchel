import { computed, ref, shallowRef, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  buildSettingsIndex,
  loadSettingsCatalogue,
  searchSettings,
  type SettingsSearchItem,
  type SettingsSearchResult,
} from '@/components/settings/settingsSearch';
import { useSettingsSections, type SettingsSection } from '@/components/settings/settingsSections';

// Shared by the sidebar and the phone overview, so a typed filter survives switching layouts.
const query = ref('');

export function useSettingsFilter() {
  const { t, locale } = useI18n();
  const { groups } = useSettingsSections();
  const index = shallowRef<Map<SettingsSection, string[]> | null>(null);
  let indexedLocale = '';

  // The parts load with the first typed letter, not with Settings.
  watch(
    [query, locale],
    async ([value, currentLocale]) => {
      if (!value.trim() || indexedLocale === currentLocale) return;
      indexedLocale = currentLocale;
      index.value = buildSettingsIndex(await loadSettingsCatalogue(), currentLocale);
    },
    { immediate: true },
  );

  const items = computed<SettingsSearchItem[]>(() =>
    groups.value.flatMap((group) => group.items.map((item) => ({
      id: item.id,
      label: item.label,
      description: t(`settings.sections.${item.id}.description`),
    }))),
  );
  const filtering = computed(() => query.value.trim().length > 0);
  const results = computed<SettingsSearchResult[]>(() =>
    filtering.value ? searchSettings(query.value, items.value, index.value ?? new Map()) : [],
  );

  return { query, filtering, results, groups };
}
