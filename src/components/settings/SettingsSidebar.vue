<script setup lang="ts">
import NavItem from '@/components/ui/NavItem.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import SearchInput from '@/components/ui/SearchInput.vue';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { registerSettingsFilterField } from '@/composables/settingsFilterFocus';
import { useSettingsFilter } from '@/composables/useSettingsFilter';
import { useSettingsLocation } from '@/composables/useSettingsLocation';
import {
  resolveSettingsSection,
  settingsSectionIcons,
  type SettingsSection,
} from './settingsSections';

const { t } = useI18n();
const { query, filtering, results, groups } = useSettingsFilter();
const location = useSettingsLocation();
const activeSection = computed(() => resolveSettingsSection(location.section.value));
const filterInput = ref<InstanceType<typeof SearchInput> | null>(null);
const highlighted = ref(0);

/** While filtering, one ranked list replaces the groups; the rows stay the same. */
const shownGroups = computed(() => (
  filtering.value
    ? [{ title: t('settings.search.results'), items: results.value }]
    : groups.value.map((group) => ({
      title: group.title,
      items: group.items.map((item) => ({ ...item, hint: null as string | null })),
    }))
));

watch(results, () => {
  highlighted.value = 0;
});

// Opened on About or Diagnostics, the list starts scrolled to the top and the
// current section sits out of sight; bring it in, without moving the page.
const root = ref<HTMLElement | null>(null);
function revealActive() {
  void nextTick(() => {
    root.value?.querySelector('[aria-current="page"]')?.scrollIntoView({ block: 'nearest' });
  });
}
onMounted(revealActive);
// Sections the shell confirms late (Shortcuts, System tray) push the rest
// down after the first scroll, so follow the list as well as the selection.
watch([activeSection, () => shownGroups.value.length, () => groups.value.flatMap((group) => group.items).length], revealActive);

const unregister = registerSettingsFilterField(() => {
  filterInput.value?.focus();
  filterInput.value?.select();
});
onBeforeUnmount(() => {
  unregister();
  // Coming back to Settings starts from the whole list.
  query.value = '';
});

function selectSection(section: SettingsSection) {
  location.show(section);
}

function onFilterKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && query.value) {
    // First Escape empties the field; the next one leaves Settings.
    event.preventDefault();
    event.stopPropagation();
    query.value = '';
    return;
  }
  if (!filtering.value) return;
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    const count = results.value.length;
    if (count === 0) return;
    highlighted.value = (highlighted.value + (event.key === 'ArrowDown' ? 1 : -1) + count) % count;
    return;
  }
  if (event.key === 'Enter') {
    const result = results.value[highlighted.value];
    if (!result) return;
    event.preventDefault();
    selectSection(result.id);
  }
}

function isHighlighted(section: SettingsSection): boolean {
  return filtering.value && results.value[highlighted.value]?.id === section;
}
</script>

<template>
  <nav ref="root" class="settings-sidebar" :aria-label="$t('common.settings')">
    <div class="settings-sidebar__head">
      <h2>{{ $t('common.settings') }}</h2>
      <div class="settings-filter">
        <SearchInput
          ref="filterInput"
          v-model="query"
          data-settings-filter
          :placeholder="$t('settings.search.placeholder')"
          :label="$t('settings.search.label')"
          aria-keyshortcuts="/"
          @keydown="onFilterKeydown"
        >
          <template #trailing><kbd v-if="!query" aria-hidden="true">/</kbd></template>
        </SearchInput>
      </div>
    </div>
    <template v-for="group in shownGroups" :key="group.title">
      <h3>{{ group.title }}</h3>
      <ul>
        <li v-for="section in group.items" :key="section.id">
          <NavItem :label="section.label" :active="activeSection === section.id"
            :class="{ highlighted: isHighlighted(section.id) }"
            :aria-current="activeSection === section.id ? 'page' : undefined"
            @click="selectSection(section.id)"
          >
            <template #icon><AppIcon :grid="24"><g v-html="settingsSectionIcons[section.id]" /></AppIcon></template>
            <span class="settings-sidebar__copy">
              <span>{{ section.label }}</span>
              <small v-if="section.hint">{{ section.hint }}</small>
            </span>
          </NavItem>
        </li>
      </ul>
    </template>
    <p v-if="filtering && results.length === 0" class="settings-filter__empty" role="status">
      {{ $t('settings.search.empty', { query: query.trim() }) }}
    </p>
  </nav>
</template>

<style scoped>
/* Title and filter stay put while the list scrolls: opened on About, the
   list scrolled them away to show the current row. */
.settings-sidebar__head {
  position: sticky;
  top: 0;
  z-index: 1;
  margin: -18px -10px 0;
  padding: 18px 10px 6px;
  background: var(--bg-sidebar);
}

.settings-sidebar {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 18px 10px 12px;
}

h2 {
  margin: 0 0 10px;
  padding: 0 10px;
  color: var(--text-primary);
  font-size: var(--font-size-xl);
  font-weight: var(--weight-bold);
  letter-spacing: -0.02em;
}

h3 {
  margin: 10px 12px 4px;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

ul {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.highlighted { background: var(--bg-hover); }

/* Scrolled into view below the pinned head, not under it. */
li { scroll-margin-top: 120px; scroll-margin-bottom: var(--space-2xl); }

.settings-sidebar__copy {
  display: grid;
  min-width: 0;
}

.settings-sidebar__copy small {
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-regular);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.settings-filter {
  margin: 0 var(--space-2xs) var(--space-1-5);
}

.settings-filter kbd {
  margin-inline-end: var(--space-xs);
  padding: 0 var(--space-1-5);
  border: var(--border-default);
  border-radius: var(--radius-sm);
  color: var(--text-tertiary);
  font-family: inherit;
  font-size: var(--font-size-xs);
  pointer-events: none;
}

.settings-filter__empty {
  margin: 8px 12px;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

@media (max-width: 1020px) {
  button {
    min-height: 48px;
    padding: 11px 13px;
    font-size: var(--font-size-md);
  }
}
</style>
