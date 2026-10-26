<script setup lang="ts">
import AddContactDialog from '@/components/contacts/AddContactDialog.vue';
import ListRowButton from '@/components/ui/ListRowButton.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import { computed, onMounted, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import Avatar from '@/components/common/Avatar.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import SearchInput from '@/components/ui/SearchInput.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import { useUiStore } from '@/stores/ui';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import {
  resolveContactsSection,
  type ContactsSection,
} from '@/components/contacts/contactNavigation';
import { useContactsStore } from '@/stores/contacts';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { useI18n } from 'vue-i18n';

interface ContactRow {
  email: string;
  name: string;
  context: string;
  meta: string;
  pinned: boolean;
}

const route = useRoute();
const router = useRouter();
const { locale, t } = useI18n();
const threads = useThreadsStore();
const addressBook = useContactsStore();
const settings = useSettingsStore();
const ui = useUiStore();
const addingContact = ref(false);
const search = ref('');

onMounted(() => {
  if (!addressBook.loaded) void addressBook.load();
});

const activeSection = computed(() => resolveContactsSection(route.query.section));
const frequentContacts = computed(() => threads.allContactSummaries);

const rowsByEmail = computed(() => {
  const rows = new Map<string, ContactRow>();

  for (const contact of addressBook.contacts) {
    const email = contact.email.trim().toLowerCase();
    if (!email) continue;
    rows.set(email, {
      email: contact.email,
      name: contact.display_name?.trim() || contact.email,
      context: contact.notes?.trim() || '',
      meta: t('contacts.meta.addressBook'),
      pinned: settings.pinnedContacts.includes(email),
    });
  }

  for (const contact of frequentContacts.value) {
    const email = contact.email.trim().toLowerCase();
    if (!email) continue;
    const previous = rows.get(email);
    rows.set(email, {
      email: contact.email,
      name: previous?.name && previous.name !== previous.email ? previous.name : contact.name,
      context: previous?.context || contact.previewText,
      meta: t('contacts.meta.conversations', contact.threadCount),
      pinned: settings.pinnedContacts.includes(email),
    });
  }

  for (const pinnedEmail of settings.pinnedContacts) {
    if (rows.has(pinnedEmail)) continue;
    rows.set(pinnedEmail, {
      email: pinnedEmail,
      name: pinnedEmail,
      context: '',
      meta: t('contacts.meta.pinned'),
      pinned: true,
    });
  }

  return rows;
});

function rowsInOrder(emails: string[]): ContactRow[] {
  const seen = new Set<string>();
  return emails.flatMap((email) => {
    const normalized = email.trim().toLowerCase();
    if (!normalized || seen.has(normalized)) return [];
    seen.add(normalized);
    const row = rowsByEmail.value.get(normalized);
    return row ? [row] : [];
  });
}

const visibleContacts = computed(() => {
  if (activeSection.value === 'address-book') {
    return rowsInOrder(addressBook.contacts.map((contact) => contact.email));
  }
  if (activeSection.value === 'frequent') {
    return rowsInOrder(frequentContacts.value.map((contact) => contact.email));
  }
  if (activeSection.value === 'pinned') {
    return rowsInOrder(settings.pinnedContacts);
  }
  return [...rowsByEmail.value.values()].sort((left, right) =>
    left.name.localeCompare(right.name, locale.value, { sensitivity: 'base' }),
  );
});

const contactGroups = computed(() => {
  const query = search.value.trim().toLocaleLowerCase(locale.value);
  const contacts = visibleContacts.value
    .filter((contact) => [contact.name, contact.email, contact.context]
      .some((value) => value.toLocaleLowerCase(locale.value).includes(query)))
    .sort((left, right) => left.name.localeCompare(right.name, locale.value, { sensitivity: 'base' }));
  const groups = new Map<string, ContactRow[]>();
  for (const contact of contacts) {
    const initial = [...contact.name.normalize('NFD').replace(/\p{M}/gu, '')][0]?.toLocaleUpperCase(locale.value) ?? '';
    const label = /\p{L}/u.test(initial) ? initial : t('contacts.otherInitial');
    const group = groups.get(label) ?? [];
    group.push(contact);
    groups.set(label, group);
  }
  return [...groups].map(([label, contacts]) => ({ label, contacts }));
});
const filteredCount = computed(() => contactGroups.value.reduce((total, group) => total + group.contacts.length, 0));

const sectionCopy = computed<Record<ContactsSection, { title: string; subtitle: string; empty: string }>>(() => ({
  all: {
    title: t('contacts.sections.all.title'),
    subtitle: t('contacts.sections.all.subtitle'),
    empty: t('contacts.sections.all.empty'),
  },
  'address-book': {
    title: t('contacts.sections.addressBook.title'),
    subtitle: t('contacts.sections.addressBook.subtitle'),
    empty: t('contacts.sections.addressBook.empty'),
  },
  frequent: {
    title: t('contacts.sections.frequent.title'),
    subtitle: t('contacts.sections.frequent.subtitle'),
    empty: t('contacts.sections.frequent.empty'),
  },
  pinned: {
    title: t('contacts.sections.pinned.title'),
    subtitle: t('contacts.sections.pinned.subtitle'),
    empty: t('contacts.sections.pinned.empty'),
  },
}));

const currentCopy = computed(() => sectionCopy.value[activeSection.value]);

function openContact(email: string) {
  threads.openContact(email);
  void router.push({ name: 'inbox' });
}
</script>

<template>
  <section class="contacts-view">
    <PaneHeader
      class="contact-header"
      :title="currentCopy.title"
      :subtitle="currentCopy.subtitle"
      :count="filteredCount"
    >
      <template #actions>
        <AppButton variant="secondary" size="sm" :aria-label="t('contacts.addressBookSettings')" @click="ui.openSettings('contacts')">
          <template #icon><AppIcon><path d="M3 2.5h10v11H3zM5.5 5h5M5.5 8h5" /></AppIcon></template>
          <span class="contact-settings-label">{{ t('contacts.addressBookSettings') }}</span>
        </AppButton>
        <AppButton variant="primary" size="sm" @click="addingContact = true">
          <template #icon><AppIcon><path d="M8 2v12M2 8h12" /></AppIcon></template>
          {{ t('contacts.add.title') }}
        </AppButton>
      </template>
    </PaneHeader>

    <div class="contact-search">
      <SearchInput id="contact-search" v-model="search" :label="t('contacts.search')" :placeholder="t('contacts.search')" />
    </div>
    <div class="contacts-view__scroll">
      <div v-if="filteredCount" class="contact-list">
        <section v-for="group in contactGroups" :key="group.label" class="contact-group" :aria-label="group.label">
          <SectionHeading :label="group.label" />
          <ListRowButton
            v-for="contact in group.contacts"
            class="contact-row"
            :key="contact.email"
            @click="openContact(contact.email)"
          >
            <Avatar :name="contact.name" :email="contact.email" :size="36" />
            <span class="copy">
              <strong>{{ contact.name }}</strong>
              <span v-if="contact.name.trim().toLowerCase() !== contact.email.trim().toLowerCase()">
                {{ contact.email }}
              </span>
              <span v-if="contact.context" class="context">{{ contact.context }}</span>
            </span>
            <span v-if="contact.pinned || activeSection === 'all'" class="row-meta">
              <span v-if="contact.pinned" :aria-label="t('nav.contactPinned')">
                <AppIcon size="sm"><path d="M6.5 2.5h3l-.4 4.1 2.4 2.1v1H4.5v-1l2.4-2.1zM8 9.7v3.8" /></AppIcon>
              </span>
              <span v-if="activeSection === 'all'" class="source-tag">{{ contact.meta }}</span>
            </span>
          </ListRowButton>
        </section>
      </div>
      <!-- A failed query leaves the list empty. Without this branch the empty
           state claims the address book was never imported. -->
      <ErrorState
        v-else-if="addressBook.error"
        :title="$t('contacts.loadFailed')"
        :detail="backendErrorLabel(addressBook.error)"
        :busy="addressBook.loading"
        @retry="addressBook.load()"
      />
      <EmptyState
        v-else-if="search.trim() && !addressBook.loading"
        :title="t('contacts.noMatches')"
        :description="t('contacts.noMatchesDescription')"
      />
      <EmptyState
        v-else-if="!addressBook.loading"
        :title="currentCopy.empty"
        :description="activeSection === 'pinned' ? t('contacts.emptyPinnedDescription') : t('contacts.emptyDescription')"
        :action-label="['pinned', 'frequent'].includes(activeSection) ? undefined : t('contacts.connectAddressBook')"
        @action="ui.openSettings('contacts')"
      >
        <template #icon>
          <AppIcon size="lg"><circle cx="6" cy="5" r="2.5" /><path d="M1.5 13v-1a4.5 4.5 0 0 1 9 0v1M11 3a2.5 2.5 0 0 1 0 5M12 10a3 3 0 0 1 2.5 3" /></AppIcon>
        </template>
      </EmptyState>
    </div>
    <AddContactDialog v-if="addingContact" @dismiss="addingContact = false" />
  </section>
</template>

<style scoped>
.contacts-view {
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr);
  grid-template-columns: minmax(0, 1fr);
  height: 100%;
  min-width: 0;
  min-height: 0;
  color: var(--text-primary);
  background: var(--bg-list);
}

.contact-header {
  max-width: calc(var(--directory-content-max) + 2 * var(--space-lg));
}

.contact-search {
  max-width: var(--directory-content-max);
  margin: 0 var(--space-lg) var(--space-md);
}

.contacts-view__scroll {
  min-height: 0;
  overflow-y: auto;
  border-top: 1px solid var(--border-color-muted);
  padding: var(--space-sm) var(--space-lg) var(--space-2xl);
}

.contact-list {
  max-width: var(--directory-content-max);
}

.contact-group + .contact-group {
  margin-top: var(--space-md);
}

.contact-group > :deep(.section-heading) {
  padding-inline: var(--space-md);
}

.contact-group .contact-row {
  display: grid;
  grid-template-columns: var(--list-avatar-size) minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-md);
  padding: var(--space-2-5) var(--space-md);
  border-bottom: 1px solid var(--border-color-muted);
  border-radius: 0;
}

.copy {
  display: grid;
  min-width: 0;
  gap: var(--space-2xs);
}

.copy strong,
.copy span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.copy > span {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-regular);
}

.copy strong {
  font-size: var(--font-size-md);
  font-weight: var(--weight-medium);
}

.copy .context {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.row-meta {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--space-1-5);
  max-width: var(--directory-provenance-max);
  margin-left: var(--space-xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-regular);
  text-align: right;
  overflow-wrap: anywhere;
}

.row-meta :deep(svg) {
  color: var(--accent-amber);
}

@media (max-width: 700px) {
  .contact-settings-label { display: none; }
  .contact-group .contact-row {
    grid-template-columns: var(--list-avatar-size) minmax(0, 1fr);
    row-gap: var(--space-2xs);
  }
  .row-meta {
    grid-column: 2;
    justify-self: start;
    flex-wrap: wrap;
    justify-content: flex-start;
    margin-left: 0;
    text-align: left;
  }
}
</style>
