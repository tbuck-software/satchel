<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { computed, defineAsyncComponent, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useSettingsStore } from '@/stores/settings';
import { useAppUpdateStore } from '@/stores/appUpdate';
import { useListSearchStore } from '@/stores/listSearch';
import { useThreadsStore, type ContactThreadSummary } from '@/stores/threads';
import { useComposerStore } from '@/stores/composer';
import { useComposeWindow } from '@/composables/useComposeWindow';
import { startWindowDrag as dragWindow } from '@/services/appWindow';
import Avatar from '@/components/common/Avatar.vue';
import SidebarMailTools from './SidebarMailTools.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { Calendar, Download, ListTodo, Mail, Pin, Settings, Users } from 'lucide';
import IconButton from '@/components/ui/IconButton.vue';
import SidebarMailNav from './SidebarMailNav.vue';
import SyncButton from './SyncButton.vue';
import { useConnectionStore } from '@/stores/connection';
const SyncStatusLine = defineAsyncComponent(() => import('./SyncStatusLine.vue'));

// Route-specific sidebars used to pull calendar grids and the complete
// settings navigation into the Inbox bundle even though their v-if was false.
const CalendarSidebar = defineAsyncComponent(() => import('@/components/calendar/CalendarSidebar.vue'));
const ContactsSidebar = defineAsyncComponent(() => import('@/components/contacts/ContactsSidebar.vue'));
const TasksSidebar = defineAsyncComponent(() => import('@/components/tasks/TasksSidebar.vue'));
const SettingsSidebar = defineAsyncComponent(() => import('@/components/settings/SettingsSidebar.vue'));
const FolderBrowserPanel = defineAsyncComponent(() => import('./FolderBrowserPanel.vue'));
// Async so the calendar utilities stay out of the inbox bundle: the card is a
// footnote in the sidebar, not a reason to parse the grid code at startup.
const SidebarAgenda = defineAsyncComponent(() => import('./SidebarAgenda.vue'));
// Standing notices live here, beside the sync button, as cards: not as a
// band across the top of the window. ConnectionBanner stays mounted (it hosts
// the certificate dialog); the reauth notice loads only when it has a case.
const ConnectionBanner = defineAsyncComponent(() => import('./ConnectionBanner.vue'));
const AccountReauthBanner = defineAsyncComponent(() => import('@/components/settings/AccountReauthBanner.vue'));

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const accounts = useAccountsStore();
const connection = useConnectionStore();
const syncStatusReady = ref(false);
watch(() => connection.syncProgress.some(row => row.backfill_remaining > 0), (active) => {
  if (active) syncStatusReady.value = true;
}, { immediate: true });
const reauthNeeded = computed(() =>
  Boolean(accounts.credentialLock) || accounts.accountsWithAuthError.length > 0);
const threads = useThreadsStore();
const settings = useSettingsStore();
const composer = useComposerStore();
const appUpdate = useAppUpdateStore();
const listSearch = useListSearchStore();
const composeWindow = useComposeWindow(router);
const foldersPanelOpen = ref(false);
const globalFrequentContacts = ref<ContactThreadSummary[]>([]);
watch(() => accounts.visibilityKey, () => { globalFrequentContacts.value = []; });

/** Pinned contacts first (in pin order), then the genuinely frequent ones. */
const frequentContacts = computed(() => {
  const summaries = threads.activeContactEmail
    ? globalFrequentContacts.value
    : threads.frequentContactSummaries;
  // Pins are cross-account: their name/preview resolves over every account,
  // not just the active one — otherwise a pinned contact from the other
  // inbox degrades to a bare address.
  const byEmail = threads.pinnedContactSummaries;
  const pinned = settings.pinnedContacts.map(
    (email) =>
      byEmail.get(email) ?? {
        email,
        name: email,
        threadCount: 0,
        messageCount: 0,
        unreadCount: 0,
        lastMessageAt: '',
        lastMessageAtMs: 0,
        previewText: t('nav.contactPinned'),
        threadIds: [],
      },
  );
  const pinnedSet = new Set(settings.pinnedContacts);
  const frequent = summaries.filter((contact) => !pinnedSet.has(contact.email.toLowerCase()));
  return [...pinned, ...frequent].slice(0, Math.max(5, pinned.length));
});

function isPinned(contact: ContactThreadSummary): boolean {
  return settings.pinnedContacts.includes(contact.email.toLowerCase());
}
async function onComposeClick(event: MouseEvent) {
  if (event.shiftKey || event.metaKey || event.ctrlKey) {
    try {
      if (await composeWindow.openNative()) return;
    } catch {
      // Fallback to in-window compose
    }
  }
  composer.openNew();
}

function goTo(name: string) {
  void router.push({ name });
}

function returnToMail() {
  // Leaving a contextual module must not silently reset the mailbox, account
  // or workflow the user had selected before opening it.
  void router.push({ name: 'inbox' });
}

function startWindowDrag(event: MouseEvent) {
  if (event.button !== 0 || !composeWindow.supportsNative()) return;
  event.preventDefault();
  void dragWindow();
}

const updateLabel = computed(() => {
  switch (appUpdate.status) {
    case 'downloading': return t('app.update.downloading', { progress: appUpdate.progress });
    case 'ready': return t('app.update.restart');
    case 'failed': return t('app.update.failedRetry');
    default: return t('app.update.available');
  }
});

const calendarModuleActive = computed(() => route.name === 'calendar');
const contactsModuleActive = computed(() => route.name === 'contacts');
const tasksModuleActive = computed(() => route.name === 'tasks');
const settingsModuleActive = computed(() => route.name === 'settings');
const contextualSidebarActive = computed(
  () => calendarModuleActive.value || contactsModuleActive.value || tasksModuleActive.value
    || settingsModuleActive.value,
);

// Reading the contact projection unguarded would rebuild it on every thread
// mutation, including on routes that never show the list.
watch(
  () => (contextualSidebarActive.value || threads.activeContactEmail ? null : threads.frequentContactSummaries),
  (contacts) => {
    // A contact filter replaces the thread store with that contact's result
    // set. Keep the sidebar based on the last unfiltered result instead of
    // turning it into a list of co-participants from the selected contact.
    if (contacts) {
      globalFrequentContacts.value = contacts;
    }
  },
  { immediate: true },
);

/**
 * A contact click opens the (FTS-backed, instant) list search for the
 * person — not the heavyweight contact filter that reloads the thread list.
 * Contacts span accounts, so it searches all mail.
 */
function openContact(contact: ContactThreadSummary) {
  listSearch.open(`from:${contact.email}`, 'all');
  void router.push({ name: 'inbox' });
}

function normalizeEmail(email?: string | null): string {
  return email?.trim().toLowerCase() ?? '';
}

function isHighlightedContact(contact: ContactThreadSummary): boolean {
  const email = normalizeEmail(contact.email);
  if (!email) {
    return false;
  }

  const activeContactEmail = normalizeEmail(threads.activeContactEmail);
  if (activeContactEmail) {
    return activeContactEmail === email;
  }

  return threads.activeThread?.participants.some((participant) => normalizeEmail(participant.email) === email) ?? false;
}
</script>

<template>
  <aside class="relative min-w-0 min-h-0 flex h-full flex-col bg-sidebar overflow-visible">
    <!-- Native desktop titlebar region — hidden on Android, iOS and web. -->
    <div class="desktop-titlebar desktop-window-chrome flex items-center px-3.5 drag-region" data-tauri-drag-region>
      <div class="w-[68px] h-3.5 shrink-0" data-tauri-drag-region />
    </div>

    <header class="sidebar-brand drag-region" data-tauri-drag-region @mousedown="startWindowDrag">
      <span
        class="sidebar-brand__icon"
        aria-hidden="true"
        data-tauri-drag-region
      />
      <h1 class="sidebar-brand__name" data-tauri-drag-region>Satchel</h1>
      <SyncButton class="sidebar-brand__sync" />
    </header>

    <SidebarMailTools @compose="onComposeClick" />

    <!-- Scrollable sidebar content -->
    <div class="sidebar-scroll min-h-0 overflow-y-auto overflow-x-hidden">
      <SettingsSidebar v-if="settingsModuleActive" />
      <CalendarSidebar v-else-if="calendarModuleActive" />
      <ContactsSidebar v-else-if="contactsModuleActive" />
      <TasksSidebar v-else-if="tasksModuleActive" />
      <template v-else>
      <SidebarMailNav @open-folders-panel="foldersPanelOpen = true" />
      <!-- Frequent contacts -->
      <nav v-if="frequentContacts.length > 0" class="grid gap-1 px-3 pb-3 pt-5">
        <div class="flex items-center justify-between px-2 pb-1">
          <span class="text-2xs font-semibold text-tertiary">{{ $t('nav.frequent') }}</span>
          <AppButton class="text-2xs" @click="goTo('contacts')" variant="ghost" size="sm">{{ $t('common.all') }}</AppButton>
        </div>
        <div class="grid gap-0.5">
          <CardButton
            v-for="contact in frequentContacts"
            :key="contact.email"
            :class="[
              'group/contact relative grid h-[34px] min-w-0 grid-cols-[20px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-sm px-2.5 text-left transition-colors hover:bg-hover',
              isHighlightedContact(contact) ? 'bg-hover' : ''
            ]"
            :title="$t('nav.contactSummary', { name: contact.name, email: contact.email, count: contact.threadCount })"
            :aria-label="$t('nav.contactLabel', { name: contact.name, count: contact.threadCount })"
            @click="openContact(contact)"
          >
            <span class="relative shrink-0">
              <Avatar :name="contact.name" :email="contact.email" :size="20" />
              <span
                v-if="contact.unreadCount > 0"
                class="absolute -left-2 top-1/2 h-1.5 w-1.5 -translate-y-1/2 rounded-full bg-accent-primary"
                aria-hidden="true"
              />
            </span>
            <span class="min-w-0 flex-1 truncate text-md font-medium text-primary">{{ contact.name }}</span>
            <span
              class="contact-pin"
              :class="{ 'contact-pin--pinned': isPinned(contact) }"
              role="button"
              tabindex="0"
              :title="isPinned(contact) ? $t('nav.unpinContact') : $t('nav.pinContact')"
              :aria-label="isPinned(contact)
                ? $t('nav.unpinContactNamed', { name: contact.name })
                : $t('nav.pinContactNamed', { name: contact.name })"
              @click.stop="settings.togglePinnedContact(contact.email)"
              @keydown.enter.stop.prevent="settings.togglePinnedContact(contact.email)"
            >
              <AppIcon :icon="Pin" size="sm" :fill="isPinned(contact) ? 'currentColor' : 'none'" />
            </span>
          </CardButton>
        </div>
      </nav>

      </template>
    </div>

    <!-- Kept outside the scroll container so every primary destination stays
         anchored to the bottom, even with a long mailbox tree. -->
    <footer class="sidebar-footer">
      <SyncStatusLine v-if="syncStatusReady" hide-idle progress-only />
      <AccountReauthBanner v-if="reauthNeeded" variant="card" />
      <ConnectionBanner variant="card" />
      <Suspense v-if="!contextualSidebarActive">
        <SidebarAgenda />
      </Suspense>
      <!-- One row: the areas on the left, the app on the right. -->
      <div class="sidebar-footer__row">
        <nav class="sidebar-footer__areas" :aria-label="$t('nav.modules')">
          <IconButton
            :title="$t('nav.mail')"
            :active="!contextualSidebarActive"
            :aria-current="!contextualSidebarActive ? 'page' : undefined"
            @click="returnToMail"
          >
            <AppIcon :icon="Mail" />
          </IconButton>
          <IconButton
            :title="$t('nav.calendar')"
            :active="calendarModuleActive"
            :aria-current="calendarModuleActive ? 'page' : undefined"
            @click="goTo('calendar')"
          >
            <AppIcon :icon="Calendar" />
          </IconButton>
          <IconButton
            :title="$t('nav.contacts')"
            :active="contactsModuleActive"
            :aria-current="contactsModuleActive ? 'page' : undefined"
            @click="goTo('contacts')"
          >
            <AppIcon :icon="Users" />
          </IconButton>
          <IconButton
            :title="$t('nav.tasks')"
            :active="tasksModuleActive"
            :aria-current="tasksModuleActive ? 'page' : undefined"
            @click="goTo('tasks')"
          >
            <AppIcon :icon="ListTodo" />
          </IconButton>
        </nav>
        <IconButton
          v-if="appUpdate.visible"
          class="sidebar-footer__update"
          tone="accent"
          :title="updateLabel"
          :disabled="appUpdate.status === 'downloading'"
          @click="void appUpdate.activate()"
        >
          <AppIcon :icon="Download" />
        </IconButton>
        <IconButton
          :title="$t('common.settings')"
          :active="settingsModuleActive"
          :aria-current="settingsModuleActive ? 'page' : undefined"
          @click="goTo('settings')"
        >
          <AppIcon :icon="Settings" />
        </IconButton>
      </div>
    </footer>

    <FolderBrowserPanel
      v-if="foldersPanelOpen"
      :open="foldersPanelOpen"
      @close="foldersPanelOpen = false"
    />
  </aside>
</template>

<style scoped>
.desktop-titlebar {
  height: var(--satchel-window-chrome-height);
  flex-shrink: 0;
}

.sidebar-brand {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: var(--space-sm);
  min-width: 0;
  padding: var(--space-lg) var(--space-md) var(--space-5);
}

.sidebar-brand__icon {
  display: block;
  flex-shrink: 0;
  width: var(--space-2xl);
  height: var(--space-2xl);
  background: var(--text-primary);
  -webkit-mask: url('/branding/satchel-mark.svg') center / contain no-repeat;
  mask: url('/branding/satchel-mark.svg') center / contain no-repeat;
  user-select: none;
}

.sidebar-brand__name {
  min-width: 0;
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: none;
}

.sidebar-brand__sync {
  margin-left: auto;
}

/* The native host already draws the traffic lights. Share their row and keep
   a clear inset for all three controls, including at the minimum pane width. */
:global(html[data-runtime-platform='macos'][data-window-chrome='true'] .desktop-titlebar) {
  display: none;
}

:global(html[data-runtime-platform='macos'][data-window-chrome='true'] .sidebar-brand) {
  height: var(--satchel-window-chrome-height);
  margin-bottom: var(--space-md);
  padding: 0 var(--space-md) 0 calc(2 * var(--space-2xl) + var(--space-xl) + var(--space-xs));
}

:global(html[data-runtime-platform='macos'][data-window-chrome='true'] .sidebar-brand__name) {
  font-size: var(--font-size-md);
}

.drag-region {
  -webkit-app-region: drag;
}

.drag-region :deep(button) {
  -webkit-app-region: no-drag;
}

.sidebar-scroll {
  flex: 1 1 0;
  scrollbar-gutter: stable;
}

.contact-pin {
  display: grid;
  place-items: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-sm);
  color: var(--text-tertiary);
  opacity: 0;
  transition: opacity var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
}

.group\/contact:hover .contact-pin,
.contact-pin:focus-visible {
  opacity: 1;
}

.contact-pin--pinned {
  opacity: 1;
  color: var(--accent-amber);
}

.contact-pin:hover {
  color: var(--text-primary);
}

.sidebar-footer {
  position: relative;
  z-index: 2;
  display: grid;
  /* A grid item is min-width:auto, so one unbreakable string in any of these
     rows pushes the whole footer — and with it the sidebar — past its width. */
  grid-template-columns: minmax(0, 1fr);
  flex: 0 0 auto;
  gap: 5px;
  padding: 8px;
  border-top: 1px solid var(--border-color-muted);
  background: var(--bg-sidebar);
}

.sidebar-footer__row {
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
  min-width: 0;
}

.sidebar-footer__areas {
  display: flex;
  flex: 1;
  align-items: center;
  gap: var(--space-2xs);
  min-width: 0;
}
</style>
