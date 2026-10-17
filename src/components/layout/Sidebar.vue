<script setup lang="ts">
import { computed, defineAsyncComponent, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useSmartFoldersStore } from '@/stores/smartFolders';
import { useSettingsStore } from '@/stores/settings';
import { useUiStore } from '@/stores/ui';
import { useThreadsStore, type ContactThreadSummary } from '@/stores/threads';
import { useComposerStore } from '@/stores/composer';
import { useComposeWindow } from '@/composables/useComposeWindow';
import { startWindowDrag as dragWindow } from '@/services/appWindow';
import Avatar from '@/components/common/Avatar.vue';
import SidebarMailTools from './SidebarMailTools.vue';
import IconButton from '@/components/ui/IconButton.vue';
import NavItem from '@/components/ui/NavItem.vue';
import PluginSlotHost from '@/components/plugins/PluginSlotHost.vue';
import MailboxTree from './MailboxTree.vue';
import SidebarUpdateAction from './SidebarUpdateAction.vue';
import SyncStatusLine from './SyncStatusLine.vue';

// Route-specific sidebars used to pull calendar grids and the complete
// settings navigation into the Inbox bundle even though their v-if was false.
const CalendarSidebar = defineAsyncComponent(() => import('@/components/calendar/CalendarSidebar.vue'));
const ContactsSidebar = defineAsyncComponent(() => import('@/components/contacts/ContactsSidebar.vue'));
const SettingsSidebar = defineAsyncComponent(() => import('@/components/settings/SettingsSidebar.vue'));
const FolderBrowserPanel = defineAsyncComponent(() => import('./FolderBrowserPanel.vue'));
// Async so the calendar utilities stay out of the inbox bundle: the card is a
// footnote in the sidebar, not a reason to parse the grid code at startup.
const SidebarAgenda = defineAsyncComponent(() => import('./SidebarAgenda.vue'));

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const accounts = useAccountsStore();
const threads = useThreadsStore();
const settings = useSettingsStore();
const ui = useUiStore();
const composer = useComposerStore();
const smartFolders = useSmartFoldersStore();
const composeWindow = useComposeWindow(router);
const foldersPanelOpen = ref(false);
const globalFrequentContacts = ref<ContactThreadSummary[]>([]);

/** Pinned contacts first (in pin order), then the genuinely frequent ones. */
const frequentContacts = computed(() => {
  const summaries = threads.activeContactEmail
    ? globalFrequentContacts.value
    : threads.contactSummaries;
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
const sourceAccounts = computed(() => accounts.accounts.filter((account) => account.kind !== 'email'));

function openSource(id: string) {
  ui.closeMobileSidebar();
  accounts.setActiveAccount(id);
  threads.workflowFilter = 'all';
  threads.setMailboxFilter({ kind: 'section', value: 'inbox' });
  void router.push({ name: 'inbox' });
}

/**
 * A smart folder is a saved search, so it belongs on the search page.
 *
 * It used to push the query into `threads.searchQuery`, which is only ever
 * applied as a lowercase substring test over the already-loaded page — the
 * operator syntax the editor advertises (`from:`, `has:attachment`, …) is
 * parsed exclusively in the backend's `parse_search_query`. A saved folder
 * therefore matched the literal string and reliably came up empty.
 */
function openSmartFolder(query: string) {
  ui.closeMobileSidebar();
  void router.push({ name: 'search', query: { q: query } });
}

async function onComposeClick(event: MouseEvent) {
  ui.closeMobileSidebar();
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
  ui.closeMobileSidebar();
  void router.push({ name });
}

function returnToMail() {
  ui.closeMobileSidebar();
  // Leaving a contextual module must not silently reset the mailbox, account
  // or workflow the user had selected before opening it.
  void router.push({ name: 'inbox' });
}

function startWindowDrag(event: MouseEvent) {
  if (event.button !== 0 || !composeWindow.supportsNative()) return;
  event.preventDefault();
  void dragWindow();
}

const calendarModuleActive = computed(() => route.name === 'calendar');
const contactsModuleActive = computed(() => route.name === 'contacts');
const settingsModuleActive = computed(() => route.name === 'settings');
const contextualSidebarActive = computed(
  () => calendarModuleActive.value || contactsModuleActive.value || settingsModuleActive.value,
);

// Reading `contactSummaries` here would rebuild the contact projection on
// every thread mutation, including on routes that never show the list.
watch(
  () => (contextualSidebarActive.value || threads.activeContactEmail ? null : threads.contactSummaries),
  (contacts) => {
    // A contact filter replaces the thread store with that contact's result
    // set. Keep the sidebar based on the last unfiltered result instead of
    // turning it into a list of co-participants from the selected contact.
    if (contacts) {
      globalFrequentContacts.value = contacts.slice(0, 5);
    }
  },
  { immediate: true },
);

/**
 * A contact click opens the (FTS-backed, instant) search scoped to the
 * person — not the heavyweight contact filter that reloads the thread list.
 */
function openContact(contact: ContactThreadSummary) {
  ui.closeMobileSidebar();
  // Contacts span accounts — start this search global (no account chip).
  void router.push({ name: 'search', query: { q: `from:${contact.email}`, global: '1' } });
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
    </header>

    <SidebarMailTools @compose="onComposeClick" />

    <!-- Scrollable sidebar content -->
    <div class="sidebar-scroll min-h-0 overflow-y-auto overflow-x-hidden">
      <SettingsSidebar v-if="settingsModuleActive" />
      <CalendarSidebar v-else-if="calendarModuleActive" />
      <ContactsSidebar v-else-if="contactsModuleActive" />
      <template v-else>
      <nav class="px-3">
        <MailboxTree @open-folders-panel="foldersPanelOpen = true" @navigate="ui.closeMobileSidebar()" />
        <PluginSlotHost slot-name="sidebar_section" />
      </nav>
      <!-- The other two areas sit with the mailboxes, where the drawing puts
           them: one list of places, not a switcher hidden in the footer. -->
      <nav class="grid gap-0.5 px-3 pt-5" :aria-label="$t('nav.modules')">
        <NavItem
          :label="$t('nav.contacts')"
          :active="contactsModuleActive"
          @click="goTo('contacts')"
        >
          <template #icon>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="5.5" cy="5.5" r="2.5" />
              <path d="M1.5 13.5C1.5 11.3 3.3 9.5 5.5 9.5C7.7 9.5 9.5 11.3 9.5 13.5M10.5 3.2A2.5 2.5 0 0 1 10.5 7.8M11.5 9.7C13.2 10.2 14.5 11.7 14.5 13.5" />
            </svg>
          </template>
        </NavItem>
        <NavItem
          :label="$t('nav.calendar')"
          :active="calendarModuleActive"
          @click="goTo('calendar')"
        >
          <template #icon>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
              <rect x="1.5" y="2.5" width="13" height="12" rx="2" />
              <path d="M1.5 6H14.5M5 1V3.5M11 1V3.5" />
            </svg>
          </template>
        </NavItem>
      </nav>
      <!-- Smart folders (saved searches) -->
      <nav v-if="smartFolders.items.length > 0" class="grid gap-0.5 px-3 pb-2 pt-5">
        <span class="text-2xs font-semibold text-tertiary px-3 mb-1">{{ $t('nav.smartFolders') }}</span>
        <NavItem
          v-for="folder in smartFolders.items"
          :key="folder.id"
          :label="folder.name"
          @click="openSmartFolder(folder.query)"
        >
          <template #icon>
            <span class="text-base leading-none">{{ folder.icon || '🔍' }}</span>
          </template>
        </NavItem>
      </nav>

      <!-- Mailbox tree -->
      <nav v-if="sourceAccounts.length" class="grid gap-1 px-2 py-3 border-t border-border-muted">
        <span class="text-2xs font-bold uppercase tracking-[0.08em] text-tertiary px-2">{{ $t('nav.feedsAndSources') }}</span>
        <NavItem v-for="source in sourceAccounts" :key="source.id" :label="source.displayName" :active="accounts.activeAccountId===source.id" @click="openSource(source.id)">
          <template #icon><span class="source-orb" :style="{background:source.color}" /></template>
        </NavItem>
      </nav>
      <!-- Frequent contacts -->
      <nav v-if="frequentContacts.length > 0" class="grid gap-1 px-3 pb-3 pt-5">
        <div class="flex items-center justify-between px-2 pb-1">
          <span class="text-2xs font-semibold text-tertiary">{{ $t('nav.frequent') }}</span>
          <button type="button" class="border-0 bg-transparent p-0 text-2xs font-medium text-tertiary hover:text-secondary" @click="goTo('contacts')">{{ $t('common.all') }}</button>
        </div>
        <div class="grid gap-0.5">
          <button
            v-for="contact in frequentContacts"
            :key="contact.email"
            type="button"
            :class="[
              'group/contact relative grid h-[34px] min-w-0 grid-cols-[20px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-sm bg-transparent px-2.5 text-left transition-colors hover:bg-hover',
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
              <svg width="12" height="12" viewBox="0 0 16 16" :fill="isPinned(contact) ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <path d="M6.5 2.5h3l-.4 4.1 2.4 2.1v1H4.5v-1l2.4-2.1z" />
                <path d="M8 9.7v3.8" />
              </svg>
            </span>
          </button>
        </div>
      </nav>

      </template>
    </div>

    <!-- Kept outside the scroll container so every primary destination stays
         anchored to the bottom, even with a long mailbox tree. -->
    <footer class="sidebar-footer">
      <SidebarUpdateAction />
      <Suspense v-if="!contextualSidebarActive">
        <SidebarAgenda />
      </Suspense>
      <div class="module-switcher">
        <button
          v-if="contextualSidebarActive"
          type="button"
          :title="$t('nav.backToMail')"
          @click="returnToMail"
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M10.5 3 5.5 8l5 5" />
          </svg>
          <span>{{ $t('common.back') }}</span>
        </button>
        <button
          v-else
          type="button"
          :class="{ active: settingsModuleActive }"
          :aria-current="settingsModuleActive ? 'page' : undefined"
          :title="$t('common.settings')"
          @click="goTo('settings')"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
            <circle cx="12" cy="12" r="3" />
          </svg>
          <span>{{ $t('common.settings') }}</span>
        </button>
        <IconButton :title="$t('commands.showShortcuts')" @click="ui.toggleShortcuts()">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="8" cy="8" r="6.5" />
            <path d="M6.2 6.1a1.9 1.9 0 1 1 2.4 2.2c-.4.2-.6.5-.6.9v.4" />
            <path d="M8 12.1h.01" />
          </svg>
        </IconButton>
      </div>
      <SyncStatusLine hide-idle />
    </footer>

    <FolderBrowserPanel
      v-if="foldersPanelOpen"
      :open="foldersPanelOpen"
      @close="foldersPanelOpen = false"
      @navigate="ui.closeMobileSidebar()"
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
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
  text-overflow: ellipsis;
  white-space: nowrap;
  user-select: none;
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
.source-orb{display:block;width:10px;height:10px;border-radius:50%;box-shadow:0 0 0 4px color-mix(in srgb,currentColor 10%,transparent)}

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

.module-switcher {
  display: flex;
  align-items: center;
  gap: var(--space-2xs);
}

.module-switcher > button {
  display: flex;
  flex-direction: row;
  align-items: center;
  justify-content: flex-start;
  gap: 10px;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 36px;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  padding: var(--space-1-5) var(--space-2-5);
  font: inherit;
  font-size: var(--font-size-md);
  font-weight: var(--weight-medium);
  text-align: left;
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.module-switcher > button svg {
  flex-shrink: 0;
}

.module-switcher > button span {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.module-switcher > button:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.module-switcher > button.active {
  background: color-mix(in srgb, var(--accent-primary) 11%, transparent);
  color: var(--accent-primary);
  font-weight: var(--weight-semibold);
}
</style>
