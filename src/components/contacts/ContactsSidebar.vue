<script setup lang="ts">
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useContactsStore } from '@/stores/contacts';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import { resolveContactsSection, type ContactsSection } from './contactNavigation';
import { useI18n } from 'vue-i18n';

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const addressBook = useContactsStore();
const settings = useSettingsStore();
const threads = useThreadsStore();
const ui = useUiStore();

const activeSection = computed(() => resolveContactsSection(route.query.section));
const allCount = computed(() => new Set([
  ...addressBook.contacts.map((contact) => contact.email.trim().toLowerCase()),
  ...threads.allContactSummaries.map((contact) => contact.email.trim().toLowerCase()),
  ...settings.pinnedContacts,
]).size);

const sections = computed<Array<{
  id: ContactsSection;
  label: string;
  count: number;
  icon: string;
}>>(() => [
  {
    id: 'all',
    label: t('contacts.sections.all.title'),
    count: allCount.value,
    icon: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/>',
  },
  {
    id: 'address-book',
    label: t('contacts.sections.addressBook.title'),
    count: addressBook.contacts.length,
    icon: '<path d="M4 4.5A2.5 2.5 0 0 1 6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5zM4 4.5v15M8 7h8M8 11h6"/>',
  },
  {
    id: 'frequent',
    label: t('contacts.sections.frequent.title'),
    count: threads.allContactSummaries.length,
    icon: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  },
  {
    id: 'pinned',
    label: t('contacts.sections.pinned.title'),
    count: settings.pinnedContacts.length,
    icon: '<path d="M9 3h6l-.8 7 4.3 3.5V15h-13v-1.5L9.8 10zM12 15v7"/>',
  },
]);

const visibleSections = computed(() => sections.value.filter(
  (section) => section.id !== 'pinned' || section.count > 0 || activeSection.value === 'pinned',
));

function selectSection(section: ContactsSection) {
  ui.closeMobileSidebar();
  void router.replace({
    name: 'contacts',
    query: section === 'all' ? {} : { section },
  });
}
</script>

<template>
  <nav class="contacts-sidebar" :aria-label="$t('contacts.title')">
    <h2>{{ $t('contacts.title') }}</h2>
    <h3>{{ $t('contacts.views') }}</h3>
    <ul>
      <li v-for="section in visibleSections" :key="section.id">
        <button
          type="button"
          :class="{ active: activeSection === section.id }"
          :aria-current="activeSection === section.id ? 'page' : undefined"
          @click="selectSection(section.id)"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" v-html="section.icon" />
          <span>{{ section.label }}</span>
          <small>{{ section.count }}</small>
        </button>
      </li>
    </ul>
  </nav>
</template>

<style scoped>
.contacts-sidebar {
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

button {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-height: 36px;
  padding: 8px 11px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-medium);
  text-align: left;
  cursor: pointer;
}

button:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

button.active {
  background: var(--bg-active);
  color: var(--text-primary);
  font-weight: var(--weight-semibold);
}

button svg {
  opacity: 0.65;
}

button.active svg {
  color: var(--accent-primary);
  opacity: 1;
}

button span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

button small {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

@media (max-width: 1020px) {
  button {
    min-height: 48px;
    padding: 11px 13px;
    font-size: var(--font-size-md);
  }
}
</style>
