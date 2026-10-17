<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useUiStore } from '@/stores/ui';
import { useThreadsStore } from '@/stores/threads';
import { useAccountsStore } from '@/stores/accounts';
import { useContactsStore } from '@/stores/contacts';
import { useSearch, type SearchResult } from '@/composables/useSearch';
import { useCommands, type OmniCommand } from '@/composables/useCommands';
import { useRecentSearches } from '@/composables/useRecentSearches';
import { useRecentCommands } from '@/composables/useRecentCommands';
import { useThreadActions } from '@/composables/useThreadActions';
import { useFocusTrap } from '@/composables/useFocusTrap';
import { reminderSuggestions, type ReminderSuggestion } from '@/utils/reminderDate';
import { formatBinding } from '@/config/keybindings';
import { rankByQuery, searchQuery } from '@/utils/searchScore';
import type { BackendContact } from '@/services/backend';
import type { Account } from '@/types/mail';
import Avatar from '@/components/common/Avatar.vue';
import TimeAgo from '@/components/common/TimeAgo.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';

const COMMAND_LIMIT_WHILE_TYPING = 5;
const RECENT_COMMAND_LIMIT = 3;
const MAIL_LIMIT = 8;
const CONTACT_LIMIT = 4;
const ACCOUNT_LIMIT = 3;
const escapeKeyLabel = 'Esc';
const showAllKeyLabel = formatBinding('mod+enter');

type OmniItem =
  | { kind: 'command'; id: string; command: OmniCommand }
  | { kind: 'recent'; id: string; recentQuery: string }
  | { kind: 'mail'; id: string; result: SearchResult }
  | { kind: 'contact'; id: string; contact: BackendContact }
  | { kind: 'account'; id: string; account: Account }
  | { kind: 'reminder'; id: string; reminder: ReminderSuggestion }
  | { kind: 'footer'; id: string };

interface OmniGroup {
  label: string;
  items: OmniItem[];
  /** Index of the first item within the flattened list. */
  start: number;
}

const { t } = useI18n();
const ui = useUiStore();
const router = useRouter();
const threads = useThreadsStore();
const accounts = useAccountsStore();
const contactsStore = useContactsStore();
const threadActions = useThreadActions();
const { query, results, loading, showLoading, errorText, warnings, hasQuery } = useSearch({ limit: 8 });
const { commands } = useCommands();
const { recent, push: pushRecent, remove: removeRecent } = useRecentSearches();
const { recentIds: recentCommandIds, record: recordCommand } = useRecentCommands();

const inputRef = ref<HTMLInputElement | null>(null);
const panelRef = ref<HTMLElement | null>(null);
const activeIndex = ref(0);
const reminderQuery = ref('');

// Tab jumps between result groups here and never leaves the search field, so
// the panel needs only the third part of the trap: the focus goes back to
// whatever the palette was opened from, not to `<body>`.
useFocusTrap(panelRef, { isOpen: () => ui.omniOpen, moveFocusIn: () => false });

const term = computed(() => query.value.trim().toLowerCase());
const inputValue = computed({
  get: () => ui.omniMode === 'reminder' ? reminderQuery.value : query.value,
  set: (value: string) => {
    if (ui.omniMode === 'reminder') reminderQuery.value = value;
    else query.value = value;
  },
});
const activeThread = computed(() =>
  threads.find(threads.activeThreadId),
);

const commandQuery = computed(() => searchQuery(query.value));

/** Recently run commands that exist right now, for the empty palette. */
const recentCommands = computed(() =>
  recentCommandIds.value
    .map((id) => commands.value.find((command) => command.id === id))
    .filter((command): command is OmniCommand => command !== undefined)
    .slice(0, RECENT_COMMAND_LIMIT),
);

/**
 * Every typed word has to match the title or the keywords, in any order. A
 * title match ranks first; among equals the more recently used command wins.
 */
function matchingCommands(...contexts: OmniCommand['context'][]): OmniCommand[] {
  const commandSearch = commandQuery.value;
  const candidates = commands.value.filter((command) => contexts.includes(command.context));
  if (commandSearch.tokens.length === 0) {
    const shownAsRecent = new Set(recentCommands.value.map((command) => command.id));
    return candidates.filter((command) => !command.searchOnly && !shownAsRecent.has(command.id));
  }
  return rankByQuery(candidates, commandSearch, recentCommandIds.value);
}

function commandItems(list: OmniCommand[]): OmniItem[] {
  return list.map((command) => ({ kind: 'command', id: `command:${command.id}`, command }));
}

const groups = computed<OmniGroup[]>(() => {
  const list: OmniGroup[] = [];
  let start = 0;
  const add = (label: string, items: OmniItem[]) => {
    if (items.length === 0) return;
    list.push({ label, items, start });
    start += items.length;
  };

  if (ui.omniMode === 'reminder') {
    add(
      ui.omniTargetThreadIds.length > 1
        ? t('nav.omni.groups.reminderForMany', { count: ui.omniTargetThreadIds.length })
        : t('nav.omni.groups.reminder'),
      reminderSuggestions(reminderQuery.value).map((reminder) => ({
        kind: 'reminder',
        id: `reminder:${reminder.id}`,
        reminder,
      })),
    );
    return list;
  }

  if (!term.value) {
    // Recents first: reopening the panel to re-run a search is the common
    // case, and eleven commands would push them below the fold.
    add(
      t('nav.omni.groups.recent'),
      recent.value.map((entry) => ({ kind: 'recent', id: `recent:${entry}`, recentQuery: entry })),
    );
    const mailLabel = activeThread.value
      ? t('nav.omni.groups.mailActionsFor', { subject: activeThread.value.subject })
      : t('nav.omni.groups.mailActions');
    add(t('nav.omni.groups.recentCommands'), commandItems(recentCommands.value));
    add(mailLabel, commandItems(matchingCommands('mail')));
    add(t('nav.omni.groups.navigation'), commandItems(matchingCommands('navigation')));
    add(t('nav.omni.groups.app'), commandItems(matchingCommands('app')));
    return list;
  }

  // A command match wins the first Enter. Otherwise typing "remi" could open
  // an email containing "reminder" instead of entering the reminder flow the
  // user explicitly addressed by name.
  add(
    t('nav.omni.groups.mailActions'),
    commandItems(matchingCommands('mail').slice(0, COMMAND_LIMIT_WHILE_TYPING)),
  );
  add(
    t('nav.omni.groups.mails'),
    results.value
      .slice(0, MAIL_LIMIT)
      .map((result) => ({ kind: 'mail', id: `mail:${result.messageId}`, result })),
  );
  add(
    t('nav.omni.groups.contacts'),
    contactsStore.contacts
      .filter((contact) =>
        `${contact.display_name ?? ''} ${contact.email}`.toLowerCase().includes(term.value),
      )
      .sort((a, b) => (b.frequency ?? 0) - (a.frequency ?? 0))
      .slice(0, CONTACT_LIMIT)
      .map((contact) => ({ kind: 'contact', id: `contact:${contact.email}`, contact })),
  );
  add(
    t('nav.omni.groups.commands'),
    commandItems(matchingCommands('navigation', 'app').slice(0, COMMAND_LIMIT_WHILE_TYPING)),
  );
  add(
    t('nav.omni.groups.accounts'),
    accounts.accounts
      .filter((account) =>
        `${account.displayName} ${account.email}`.toLowerCase().includes(term.value),
      )
      .slice(0, ACCOUNT_LIMIT)
      .map((account) => ({ kind: 'account', id: `account:${account.id}`, account })),
  );
  if (results.value.length > 0) {
    add('', [{ kind: 'footer', id: 'footer:show-all' }]);
  }
  return list;
});

const flatItems = computed(() => groups.value.flatMap((group) => group.items));

const activeId = computed(() =>
  flatItems.value.length > 0 ? `omni-item-${activeIndex.value}` : undefined,
);

watch(flatItems, (items) => {
  if (activeIndex.value >= items.length) {
    activeIndex.value = 0;
  }
});

watch(inputValue, () => {
  activeIndex.value = 0;
});

watch(
  () => ui.omniMode,
  async (mode) => {
    if (mode === 'reminder') reminderQuery.value = '';
    activeIndex.value = 0;
    await nextTick();
    inputRef.value?.focus();
  },
);

watch(
  () => ui.omniOpen,
  async (open) => {
    if (!open) {
      // Recents cover re-running; a stale query reopening later is confusing.
      query.value = '';
      activeIndex.value = 0;
      return;
    }
    // If the user promotes this query to the full search page, navigation can
    // reuse the chunk instead of pausing after Enter.
    void import('@/views/SearchView.vue');
    await nextTick();
    inputRef.value?.focus();
    inputRef.value?.select();
  },
  { immediate: true },
);

watch(activeIndex, async () => {
  await nextTick();
  if (activeId.value) {
    document.getElementById(activeId.value)?.scrollIntoView({ block: 'nearest' });
  }
});

function move(delta: number) {
  const count = flatItems.value.length;
  if (count === 0) return;
  activeIndex.value = (activeIndex.value + delta + count) % count;
}

function jumpGroup(direction: 1 | -1) {
  const starts = groups.value.map((group) => group.start);
  if (starts.length === 0) return;
  if (direction === 1) {
    activeIndex.value = starts.find((start) => start > activeIndex.value) ?? (starts[0] ?? activeIndex.value);
    return;
  }
  const currentStart =
    [...starts].reverse().find((start) => start <= activeIndex.value) ?? (starts[0] ?? activeIndex.value);
  activeIndex.value =
    [...starts].reverse().find((start) => start < currentStart) ?? (starts.at(-1) ?? activeIndex.value);
}

function close() {
  ui.closeOmni();
}

function showAll() {
  const raw = query.value.trim();
  if (!raw) return;
  pushRecent(raw);
  close();
  void router.push({ name: 'search', query: { q: raw } });
}

function openSearchWith(raw: string) {
  pushRecent(raw);
  close();
  void router.push({ name: 'search', query: { q: raw } });
}

function activate(item: OmniItem) {
  switch (item.kind) {
    case 'command':
      recordCommand(item.command.id);
      void item.command.run();
      if (ui.omniMode === 'default') close();
      break;
    case 'recent':
      openSearchWith(item.recentQuery);
      break;
    case 'mail': {
      const raw = query.value.trim();
      if (raw) pushRecent(raw);
      threads.selectThread(item.result.threadId);
      close();
      void router.push({ name: 'thread', params: { id: item.result.threadId } });
      break;
    }
    case 'contact':
      openSearchWith(`from:${item.contact.email}`);
      break;
    case 'account':
      accounts.setActiveAccount(item.account.id);
      threads.workflowFilter = 'all';
      threads.setMailboxFilter({ kind: 'section', value: 'inbox' });
      close();
      void router.push({ name: 'inbox' });
      break;
    case 'reminder':
      void threadActions.snoozeAt(
        [...ui.omniTargetThreadIds],
        item.reminder.date.toISOString(),
      );
      close();
      break;
    case 'footer':
      showAll();
      break;
  }
}

function onEnter(event: KeyboardEvent) {
  if (ui.omniMode === 'default' && (event.metaKey || event.ctrlKey)) {
    showAll();
    return;
  }
  const item = flatItems.value[activeIndex.value];
  if (item) {
    activate(item);
  } else if (hasQuery.value) {
    showAll();
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="ui.omniOpen" class="backdrop" @click.self="close()">
        <section ref="panelRef" class="panel" role="dialog" aria-modal="true" :aria-label="$t('nav.omni.ariaLabel')">
          <div class="search-row">
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" opacity="0.4" aria-hidden="true">
              <circle cx="7" cy="7" r="5"/>
              <path d="M11 11L14 14"/>
            </svg>
            <input
              ref="inputRef"
              v-model="inputValue"
              type="search"
              :placeholder="ui.omniMode === 'reminder'
                ? $t('nav.omni.reminderPlaceholder')
                : $t('nav.omni.placeholder')"
              :aria-label="$t('nav.omni.queryLabel')"
              role="combobox"
              aria-expanded="true"
              aria-controls="omni-list"
              :aria-activedescendant="activeId"
              @keydown.down.prevent="move(1)"
              @keydown.up.prevent="move(-1)"
              @keydown.tab.exact.prevent="jumpGroup(1)"
              @keydown.shift.tab.prevent="jumpGroup(-1)"
              @keydown.enter.prevent="onEnter"
            />
            <span
              class="spinner-slot"
              :class="{ 'spinner-slot--visible': ui.omniMode === 'default' && showLoading }"
              :aria-label="ui.omniMode === 'default' && showLoading ? $t('nav.omni.searching') : undefined"
              aria-live="polite"
            />
          </div>

          <div class="results-region">
            <p v-for="warning in ui.omniMode === 'default' ? warnings : []" :key="warning" class="warning" role="status">
              {{ warning }}
            </p>

            <p v-if="ui.omniMode === 'default' && errorText" class="state state--error" role="alert">{{ errorText }}</p>

            <div v-else-if="flatItems.length > 0" id="omni-list" class="list" role="listbox" :aria-label="$t('nav.omni.results')">
            <template v-for="group in groups" :key="group.label || 'footer'">
              <SectionHeading v-if="group.label" :label="group.label" role="presentation" class="group-heading" />
              <button
                v-for="(item, offset) in group.items"
                :id="`omni-item-${group.start + offset}`"
                :key="item.id"
                type="button"
                role="option"
                :aria-selected="group.start + offset === activeIndex"
                :aria-keyshortcuts="item.kind === 'command' ? item.command.shortcut?.aria : undefined"
                class="row"
                :class="{ active: group.start + offset === activeIndex, 'row--command': item.kind === 'command', 'row--reminder': item.kind === 'reminder', 'row--footer': item.kind === 'footer' }"
                @click="activate(item)"
                @mouseenter="activeIndex = group.start + offset"
              >
                <template v-if="item.kind === 'command'">
                  <span class="row-icon" aria-hidden="true">
                    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                      <path d="M6 3L11 8L6 13"/>
                    </svg>
                  </span>
                  <span class="row-title">{{ item.command.title }}</span>
                  <kbd v-if="item.command.shortcut" class="command-shortcut">{{ item.command.shortcut.label }}</kbd>
                </template>

                <template v-else-if="item.kind === 'recent'">
                  <span class="row-icon" aria-hidden="true">
                    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
                      <circle cx="8" cy="8" r="5.5"/>
                      <path d="M8 5.5V8L10 9.5"/>
                    </svg>
                  </span>
                  <span class="row-title">{{ item.recentQuery }}</span>
                  <!-- span, not button: options must not nest interactive elements -->
                  <span
                    class="recent-remove"
                    role="button"
                    tabindex="-1"
                    :aria-label="$t('nav.omni.removeRecentNamed', { query: item.recentQuery })"
                    :title="$t('nav.omni.removeRecent')"
                    @click.stop="removeRecent(item.recentQuery)"
                  >
                    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
                      <path d="M4 4L12 12M12 4L4 12"/>
                    </svg>
                  </span>
                </template>

                <template v-else-if="item.kind === 'mail'">
                  <Avatar :name="item.result.fromName" :email="item.result.fromAddress" :size="30" />
                  <span class="hit-body">
                    <span class="hit-top">
                      <strong :class="{ unread: !item.result.isRead }">{{ item.result.fromName }}</strong>
                      <TimeAgo :date="item.result.date" class="hit-time" />
                    </span>
                    <span class="hit-subject">{{ item.result.subject }}</span>
                    <span v-if="item.result.snippet !== item.result.subject" class="snippet">
                      <template v-for="(segment, index) in item.result.snippetSegments" :key="index">
                        <mark v-if="segment.marked">{{ segment.text }}</mark>
                        <template v-else>{{ segment.text }}</template>
                      </template>
                    </span>
                  </span>
                </template>

                <template v-else-if="item.kind === 'contact'">
                  <Avatar :name="item.contact.display_name ?? item.contact.email" :email="item.contact.email" :size="30" />
                  <span class="hit-body">
                    <span class="row-title">{{ item.contact.display_name || item.contact.email }}</span>
                    <span class="row-subtitle">{{ item.contact.email }}</span>
                  </span>
                </template>

                <template v-else-if="item.kind === 'account'">
                  <Avatar :name="item.account.displayName" :email="item.account.email" :size="30" />
                  <span class="hit-body">
                    <span class="row-title">{{ item.account.displayName }}</span>
                    <span class="row-subtitle">{{ $t('nav.omni.activateAccount') }} · {{ item.account.email }}</span>
                  </span>
                </template>

                <template v-else-if="item.kind === 'reminder'">
                  <span class="reminder-icon" aria-hidden="true">
                    <svg viewBox="0 0 20 20"><path d="M10 2.5v2M10 15.5v2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M2.5 10h2M15.5 10h2"/><circle cx="10" cy="10" r="4.5"/><path d="M10 7.5v2.8l1.8 1"/></svg>
                  </span>
                  <span class="reminder-copy">
                    <strong>{{ item.reminder.label }}</strong>
                    <small>{{ item.reminder.detail }}</small>
                  </span>
                  <kbd class="command-shortcut">↵</kbd>
                </template>

                <template v-else>
                  <span class="footer-label">{{ $t('nav.omni.showAll') }}</span>
                  <kbd class="footer-kbd">{{ showAllKeyLabel }}</kbd>
                </template>
              </button>
            </template>
            </div>

            <p v-else-if="ui.omniMode === 'reminder'" class="state state--hint">
              {{ $t('nav.omni.noReminderDate') }}
            </p>
            <p v-else-if="showLoading" class="state">{{ $t('nav.omni.searchingState') }}</p>
            <p v-else-if="loading" class="state state--waiting" aria-hidden="true">&nbsp;</p>
            <p v-else-if="hasQuery" class="state">{{ $t('nav.omni.noResults') }}</p>
            <p v-else class="state state--hint">
              {{ $t('nav.omni.hint') }}
            </p>
          </div>

          <footer class="hints" aria-hidden="true">
            <span><kbd>↑</kbd><kbd>↓</kbd> {{ $t('nav.omni.hints.navigate') }}</span>
            <span v-if="ui.omniMode === 'default'"><kbd>⇥</kbd> {{ $t('nav.omni.hints.group') }}</span>
            <span><kbd>↵</kbd> {{ $t('common.open') }}</span>
            <span v-if="ui.omniMode === 'default'"><kbd>{{ showAllKeyLabel }}</kbd> {{ $t('nav.omni.hints.allHits') }}</span>
            <span><kbd>{{ escapeKeyLabel }}</kbd> {{ ui.omniMode === 'reminder' ? $t('common.back') : $t('common.close') }}</span>
          </footer>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  background: var(--bg-backdrop);
  backdrop-filter: blur(4px);
  display: grid;
  place-items: start center;
  padding-top: 10vh;
  z-index: 100;
}

.panel {
  width: min(680px, 92vw);
  height: min(640px, 80vh);
  background: var(--bg-surface);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr) auto;
}

.results-region {
  min-height: 0;
  overflow-y: auto;
  scrollbar-gutter: stable;
}

.search-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border-color-muted);
}

.search-row input {
  flex: 1;
  border: 0;
  background: transparent;
  padding: 0;
  font-size: var(--font-size-lg);
  outline: none;
  min-width: 0;
}

.search-row input::placeholder {
  color: var(--text-tertiary);
}

.spinner-slot {
  width: 13px;
  height: 13px;
  flex-shrink: 0;
  border: 2px solid var(--border-color);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  opacity: 0;
}

.spinner-slot--visible {
  opacity: 1;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

@media (prefers-reduced-motion: reduce) {
  .spinner-slot--visible { animation-duration: 2s; }
}

.list {
  padding: 6px;
  display: grid;
  gap: 2px;
}

.group-heading {
  padding: 8px 10px 2px;
}

.row {
  width: 100%;
  text-align: left;
  border: 0;
  background: transparent;
  border-radius: var(--radius-sm);
  padding: 8px 10px;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  font: inherit;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background var(--duration-fast) var(--ease-out);
}

.row.active,
.row:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.row--command {
  grid-template-columns: auto minmax(0, 1fr) auto;
}

.row--reminder {
  grid-template-columns: auto minmax(0, 1fr) auto;
  min-height: 58px;
}

.reminder-icon {
  display: grid;
  width: 34px;
  height: 34px;
  place-items: center;
  border: 1px solid color-mix(in srgb, var(--accent-purple) 35%, var(--border-color));
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-purple) 12%, var(--bg-base));
  color: var(--accent-purple);
}

.reminder-icon svg {
  width: 18px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.reminder-copy {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.reminder-copy strong {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-strong);
}

.reminder-copy small {
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.command-shortcut {
  min-width: 24px;
  color: var(--text-tertiary);
  font-weight: var(--weight-strong);
}

/* Recent rows carry a third column for the remove control. */
.row:has(.recent-remove) {
  grid-template-columns: auto minmax(0, 1fr) auto;
}

.recent-remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-xs);
  color: var(--text-tertiary);
  opacity: 0;
  transition:
    opacity var(--duration-fast) var(--ease-out),
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.row.active .recent-remove,
.row:hover .recent-remove {
  opacity: 1;
}

.recent-remove:hover {
  background: var(--bg-active);
  color: var(--text-primary);
}

.row-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  color: var(--text-tertiary);
}

.row-title {
  font-size: var(--font-size-md);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row-subtitle {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hit-body {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.hit-top {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
}

.hit-top strong {
  font-size: var(--font-size-md);
  font-weight: var(--weight-semibold);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hit-top strong.unread {
  font-weight: var(--weight-bold);
  color: var(--text-primary);
}

.hit-time {
  flex-shrink: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.hit-subject,
.snippet {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hit-subject {
  font-size: var(--font-size-sm);
  color: var(--text-primary);
}

.snippet {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.snippet mark {
  background: transparent;
  color: var(--accent-primary);
  font-weight: var(--weight-strong);
}

.row--footer {
  grid-template-columns: minmax(0, 1fr) auto;
  justify-items: start;
}

.footer-label {
  color: var(--accent-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.footer-kbd {
  justify-self: end;
}

.warning {
  margin: 0;
  padding: 6px 16px;
  border-bottom: 1px solid var(--border-color-muted);
  color: var(--accent-amber);
  font-size: var(--font-size-xs);
}

.state {
  margin: 0;
  padding: 24px 16px;
  text-align: center;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.state--error {
  color: var(--accent-red);
}

.state--hint {
  color: var(--text-tertiary);
}

.state--waiting {
  min-height: 66px;
}

.hints {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 8px 16px;
  border-top: 1px solid var(--border-color-muted);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.hints span {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

kbd {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  padding: 1px 5px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-xs);
  background: var(--bg-raised);
  color: var(--text-secondary);
  font-family: inherit;
  font-size: var(--font-size-2xs);
}

/* A phone has no room for a dialog floating a tenth of the way down with a
   hint row underneath: the results were cut off and the hints pushed off the
   edge. Full bleed instead, like every other modal — the search *is* the
   screen while it is open. */
@media (max-width: 1020px) {
  .backdrop {
    padding-top: 0;
    place-items: stretch;
    background: var(--bg-base);
    backdrop-filter: none;
  }

  .panel {
    width: 100%;
    height: 100%;
    padding:
      var(--satchel-safe-area-top)
      var(--satchel-safe-area-right)
      var(--satchel-safe-area-bottom)
      var(--satchel-safe-area-left);
    border: 0;
    border-radius: 0;
    box-shadow: none;
    grid-template-rows: auto minmax(0, 1fr) auto;
    /* Grid tracks size to their content by default, so the hint row — four
       items on one line — pushed the panel wider than the screen and both
       edges were clipped. minmax(0, 1fr) lets the column be narrower than
       what is in it. */
    grid-template-columns: minmax(0, 1fr);
  }

  /* Four keyboard hints do not fit a phone width, and none of them describe a
     gesture that exists there. */
  .hints {
    display: none;
  }
}
</style>
