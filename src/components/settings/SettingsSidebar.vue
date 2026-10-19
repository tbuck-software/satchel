<script setup lang="ts">
import NavItem from '@/components/ui/NavItem.vue';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { useRoute, useRouter } from 'vue-router';
import { useUiStore } from '@/stores/ui';
import { registerSettingsFilterField } from '@/composables/settingsFilterFocus';
import { useSettingsFilter } from '@/composables/useSettingsFilter';
import {
  resolveSettingsSection,
  settingsSectionIcons,
  type SettingsSection,
} from './settingsSections';

const route = useRoute();
const router = useRouter();
const ui = useUiStore();
const { t } = useI18n();
const { query, filtering, results, groups } = useSettingsFilter();
const activeSection = computed(() => resolveSettingsSection(route.query.section));
const filterInput = ref<HTMLInputElement | null>(null);
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
  ui.closeMobileSidebar();
  void router.replace({ name: 'settings', query: { section } });
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
  <nav class="settings-sidebar" :aria-label="$t('common.settings')">
    <h2>{{ $t('common.settings') }}</h2>
    <label class="settings-filter">
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
        <circle cx="7" cy="7" r="5" />
        <path d="M11 11L14 14" />
      </svg>
      <input
        ref="filterInput"
        v-model="query"
        type="search"
        data-settings-filter
        :placeholder="$t('settings.search.placeholder')"
        :aria-label="$t('settings.search.label')"
        aria-keyshortcuts="/"
        @keydown="onFilterKeydown"
      />
      <kbd v-if="!query" aria-hidden="true">/</kbd>
    </label>
    <template v-for="group in shownGroups" :key="group.title">
      <h3>{{ group.title }}</h3>
      <ul>
        <li v-for="section in group.items" :key="section.id">
          <NavItem :label="section.label" :active="activeSection === section.id"
            :class="{ highlighted: isHighlighted(section.id) }"
            :aria-current="activeSection === section.id ? 'page' : undefined"
            @click="selectSection(section.id)"
          >
            <template #icon><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" v-html="settingsSectionIcons[section.id]" /></template>
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

/* The input is the box, so it keeps the focus ring base.css gives it. */
.settings-filter {
  position: relative;
  display: block;
  margin: 0 2px 6px;
  color: var(--text-tertiary);
}

.settings-filter svg {
  position: absolute;
  top: 50%;
  left: var(--space-md);
  transform: translateY(-50%);
  pointer-events: none;
}

.settings-filter input {
  width: 100%;
  height: 36px;
  padding: 0 34px 0 36px;
  border: 0;
  border-radius: var(--radius-md);
  background: var(--bg-raised);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}

.settings-filter input::placeholder {
  color: var(--text-tertiary);
}

.settings-filter kbd {
  position: absolute;
  top: 50%;
  right: var(--space-sm);
  transform: translateY(-50%);
  padding: 0 6px;
  border: 1px solid var(--border-color);
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
