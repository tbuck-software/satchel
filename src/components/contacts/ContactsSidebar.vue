<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue';
import NavItem from '@/components/ui/NavItem.vue';
import { computed } from 'vue';
import { BookUser, Pin, RotateCcwClock, Users, type IconNode } from 'lucide';
import { useRoute, useRouter } from 'vue-router';
import { useContactsStore } from '@/stores/contacts';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { resolveContactsSection, type ContactsSection } from './contactNavigation';
import { useI18n } from 'vue-i18n';

const route = useRoute();
const router = useRouter();
const { t } = useI18n();
const addressBook = useContactsStore();
const settings = useSettingsStore();
const threads = useThreadsStore();

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
  icon: IconNode;
}>>(() => [
  {
    id: 'all',
    label: t('contacts.sections.all.title'),
    count: allCount.value,
    icon: Users,
  },
  {
    id: 'address-book',
    label: t('contacts.sections.addressBook.title'),
    count: addressBook.contacts.length,
    icon: BookUser,
  },
  {
    id: 'frequent',
    label: t('contacts.sections.frequent.title'),
    count: threads.allContactSummaries.length,
    icon: RotateCcwClock,
  },
  {
    id: 'pinned',
    label: t('contacts.sections.pinned.title'),
    count: settings.pinnedContacts.length,
    icon: Pin,
  },
]);

const visibleSections = computed(() => sections.value.filter(
  (section) => section.id !== 'pinned' || section.count > 0 || activeSection.value === 'pinned',
));

function selectSection(section: ContactsSection) {
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
        <NavItem :label="section.label" :active="activeSection === section.id" :count="section.count"

          :aria-current="activeSection === section.id ? 'page' : undefined"
          @click="selectSection(section.id)"
        >
          <template #icon><AppIcon :icon="section.icon" /></template>

        </NavItem>
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

@media (max-width: 1020px) {
  button {
    min-height: 48px;
    padding: 11px 13px;
    font-size: var(--font-size-md);
  }
}
</style>
