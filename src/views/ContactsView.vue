<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import Avatar from '@/components/common/Avatar.vue';
import ErrorState from '@/components/ui/ErrorState.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
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
      :title="currentCopy.title"
      :subtitle="currentCopy.subtitle"
      :count="visibleContacts.length"
    />

    <div class="contacts-view__scroll">
      <div v-if="visibleContacts.length" class="contact-list">
        <button
          v-for="contact in visibleContacts"
          :key="contact.email"
          type="button"
          @click="openContact(contact.email)"
        >
          <Avatar :name="contact.name" :email="contact.email" :size="42" />
          <span class="copy">
            <strong>{{ contact.name }}</strong>
            <span v-if="contact.name.trim().toLowerCase() !== contact.email.trim().toLowerCase()">
              {{ contact.email }}
            </span>
            <span v-if="contact.context" class="context">{{ contact.context }}</span>
          </span>
          <span class="row-meta">
            <svg v-if="contact.pinned" width="12" height="12" viewBox="0 0 16 16" fill="currentColor" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" :aria-label="$t('nav.contactPinned')">
              <path d="M6.5 2.5h3l-.4 4.1 2.4 2.1v1H4.5v-1l2.4-2.1z" />
              <path d="M8 9.7v3.8" fill="none" />
            </svg>
            <span>{{ contact.meta }}</span>
          </span>
        </button>
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
      <p v-else class="empty">{{ currentCopy.empty }}</p>
    </div>
  </section>
</template>

<style scoped>
.contacts-view {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  height: 100%;
  min-width: 0;
  min-height: 0;
  color: var(--text-primary);
  background: var(--bg-base);
}

.contacts-view__scroll {
  min-height: 0;
  overflow-y: auto;
  border-top: 1px solid var(--border-color-muted);
  padding: 24px 28px 48px;
}

.contact-list {
  display: grid;
  gap: 6px;
  width: min(100%, 760px);
  margin: 0 auto;
}

.contact-list button {
  display: grid;
  grid-template-columns: 42px minmax(0, 1fr) auto;
  align-items: center;
  gap: 14px;
  width: 100%;
  min-width: 0;
  padding: 12px 14px;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    border-color var(--duration-fast) var(--ease-out),
    background var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
}

.contact-list button:hover {
  border-color: var(--border-color);
  background: var(--bg-hover);
  transform: translateX(2px);
}

.copy {
  display: grid;
  min-width: 0;
  gap: 2px;
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
}

.copy .context {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.row-meta {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  white-space: nowrap;
}

.row-meta svg {
  color: var(--accent-amber);
}

.empty {
  max-width: 680px;
  margin: 60px auto;
  color: var(--text-tertiary);
  text-align: center;
}

@media (max-width: 700px) {
  .contacts-view__scroll {
    padding: 16px 12px 36px;
  }

  .contact-list button {
    grid-template-columns: 42px minmax(0, 1fr);
  }

  .row-meta {
    grid-column: 2;
    justify-content: flex-start;
  }
}
</style>
