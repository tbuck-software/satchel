<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
import { useAccountsStore } from '@/stores/accounts';
import { useMailboxesStore, type FavoriteFolderGroup, type FolderTreeNode } from '@/stores/mailboxes';
import { useThreadsStore, type MailboxFilter } from '@/stores/threads';
import { useConnectionStore } from '@/stores/connection';
import { useOutboxStore } from '@/stores/outbox';
import { useThreadActions } from '@/composables/useThreadActions';
import { matchesWorkflowFilter, type WorkflowFilter } from '@/types/workflow';
import { MAILBOX_SECTIONS, type CounterMode, type SectionSpec } from '@/config/mailboxSections';
import { showMailList } from '@/router/mailViews';
import NavItem from '@/components/ui/NavItem.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import { outboxVisibleCount } from '@/utils/outboxLabels';
import { activeMailboxNavNode } from '@/utils/mailboxNav';
import { draggedThreadIds, isThreadDrag } from '@/utils/threadDrag';

type ResolvedSection = SectionSpec & {
  total: number;
  showTotal: boolean;
};

type FavoriteRow = {
  id: string;
  node: FolderTreeNode;
  depth: number;
  accountId: string;
  accountEmail: string;
};

const emit = defineEmits<{
  openFoldersPanel: [];
  navigate: [];
}>();

const router = useRouter();
const accounts = useAccountsStore();
const mailboxes = useMailboxesStore();
const threads = useThreadsStore();
const threadActions = useThreadActions();
const connection = useConnectionStore();
const outbox = useOutboxStore();

const openGroups = ref<Record<string, boolean>>({
  inbox: true,
  sent: true,
  archive: true,
  folders: true,
});

const resolvedSections = computed<ResolvedSection[]>(() =>
  MAILBOX_SECTIONS
    .filter((section) => section.forceVisible || sectionHasContent(section))
    .map((section) => {
      const total = sectionCount(section);
      return {
        ...section,
        total,
        showTotal: section.counterMode !== 'none' && total > 0,
      };
    }),
);

const favoriteFolderGroups = computed(() =>
  mailboxes.favoriteFolderGroups.filter(group => accounts.activeAccountId ? group.accountId === accounts.activeAccountId : accounts.isVisible(group.accountId)).map((group) => ({
    ...group,
    rows: flattenFavoriteTree(group, group.tree),
  })),
);

const activeNodeId = computed(() =>
  activeMailboxNavNode({
    routeName: typeof router.currentRoute.value.name === 'string' ? router.currentRoute.value.name : null,
    workflowFilter: threads.workflowFilter,
    mailboxFilter: threads.mailboxFilter,
    activeAccountId: accounts.activeAccountId,
  }),
);

function countWorkflowThreads(state: WorkflowFilter, accountId?: string | null): number {
  return threads.threads.filter((thread) => {
    const accountMatch = accountId ? thread.accountId === accountId : accounts.isVisible(thread.accountId);
    return accountMatch && matchesWorkflowFilter(thread.workflowState, state);
  }).length;
}

function countForMailboxType(mailboxType: string, mode: CounterMode, accountId?: string | null): number {
  if (mode === 'none') return 0;
  if (mode === 'threads') {
    return mailboxes.threadCountForMailboxType(mailboxType, accountId);
  }
  return mailboxes.countForMailboxType(mailboxType, accountId);
}

function sectionHasContent(section: SectionSpec): boolean {
  if (section.source === 'route') {
    return router.currentRoute.value.name === section.routeName || sectionCount(section) > 0;
  }
  if (section.source === 'workflow') {
    return countWorkflowThreads(section.workflowState!) > 0;
  }
  return mailboxes.hasMailboxType(section.mailboxType!);
}

function sectionCount(section: SectionSpec, accountId?: string | null): number {
  if (section.source === 'route') {
    return outboxVisibleCount(connection.pendingSends, Math.max(connection.failedSends, outbox.failedSendCount));
  }
  if (section.source === 'workflow') {
    return countWorkflowThreads(section.workflowState!, accountId);
  }
  return countForMailboxType(section.mailboxType!, section.counterMode, accountId);
}

function isOpen(id: string, fallback = false): boolean {
  const current = openGroups.value[id];
  if (typeof current === 'boolean') {
    return current;
  }
  return fallback;
}

function toggleOpen(id: string, fallback = false) {
  openGroups.value[id] = !isOpen(id, fallback);
}

function flattenFavoriteTree(group: FavoriteFolderGroup, nodes: FolderTreeNode[], depth = 0): FavoriteRow[] {
  return nodes.flatMap((node) => {
    const row: FavoriteRow = {
      id: `favorite:${group.accountId}:${node.path}`,
      node,
      depth,
      accountId: group.accountId,
      accountEmail: group.accountEmail,
    };

    if (!node.children.length || !isOpen(`folder:${group.accountId}:${node.path}`, true)) {
      return [row];
    }

    return [row, ...flattenFavoriteTree(group, node.children, depth + 1)];
  });
}

function resetToInboxView() {
  emit('navigate');
  void router.push({ name: 'inbox' });
}

/*
 * `null` means "leave the account alone", not "all accounts": the pill strip
 * over the list owns that scope now, and a section reset would have undone
 * the pill on every click.
 */
function applyMailboxSelection(accountId: string | null, filter: MailboxFilter) {
  if (accountId !== null) accounts.setActiveAccount(accountId);
  showMailList('all', filter);
  resetToInboxView();
}

function applyWorkflowSelection(accountId: string | null, workflowState: WorkflowFilter) {
  if (accountId !== null) accounts.setActiveAccount(accountId);
  showMailList(workflowState, { kind: 'all', value: null });
  resetToInboxView();
}

function selectSection(section: SectionSpec, accountId: string | null = null) {
  if (section.source === 'route') {
    emit('navigate');
    void router.push({ name: section.routeName });
    return;
  }
  if (section.source === 'workflow') {
    applyWorkflowSelection(accountId, section.workflowState!);
    return;
  }
  applyMailboxSelection(accountId, { kind: 'section', value: section.mailboxType! });
}

function selectFavoriteFolder(accountId: string, path: string) {
  applyMailboxSelection(accountId, { kind: 'folder', value: path });
}

const dragHoverFolderKey = ref<string>('');

function onDragLeave(key: string) {
  if (dragHoverFolderKey.value === key) {
    dragHoverFolderKey.value = '';
  }
}

function onDragOverFolder(key: string, event: DragEvent) {
  dragHoverFolderKey.value = key;
  // The row allows copy as well (for the composer); a folder always moves.
  if (event.dataTransfer && isThreadDrag(event)) event.dataTransfer.dropEffect = 'move';
}

async function onDropToFolder(folderId: string, event: DragEvent) {
  dragHoverFolderKey.value = '';
  if (!folderId) return;
  const ids = draggedThreadIds(event);
  if (ids.length === 0) return;
  const target = mailboxes.browsableFolders.find((folder) => folder.folderId === folderId);
  await threadActions.moveToFolder(ids, folderId, target?.label);
}

</script>

<template>
  <section class="grid gap-0.5 content-start">
    <!-- Mailbox sections (Inbox, Drafts, Sent, etc.) -->
    <div
      v-for="section in resolvedSections"
      :key="section.id"
      class="grid gap-0.5"
    >
      <div>
        <NavItem
          :label="$t(section.labelKey)"
          :title="section.hintKey ? $t(section.hintKey) : undefined"
          :active="activeNodeId === `section:${section.id}`"
          :count="section.showTotal ? section.total : undefined"
          :count-tone="section.id === 'outbox' && (connection.failedSends > 0 || outbox.failedSendCount > 0) ? 'danger' : 'neutral'"
          @click="selectSection(section)"
        >
          <template #icon>
            <svg
              v-if="section.icon === 'inbox'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M2 5.5h12" /><path d="M3 5.5v6.5A1.5 1.5 0 004.5 13.5h7A1.5 1.5 0 0013 12V5.5" /><path d="M5.5 8.5h5" />
            </svg>
            <svg
              v-else-if="section.icon === 'drafts'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M2 4.5h12v8.5a1 1 0 01-1 1H3a1 1 0 01-1-1z" /><path d="M2 4.5L8 9l6-4.5" />
            </svg>
            <svg
              v-else-if="section.icon === 'sent'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M14 2L2.5 7.5l4 1L8 13.5 14 2Z" /><path d="M6.5 8.5L14 2" />
            </svg>
            <svg
              v-else-if="section.icon === 'outbox'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M2 11.5h12v2H2z" /><path d="M8 12V3" /><path d="M4.8 6.2L8 3l3.2 3.2" />
            </svg>
            <svg
              v-else-if="section.icon === 'trash'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M2.5 4h11" /><path d="M6 2.5h4" /><path d="M4 4l.6 9a1 1 0 001 1h4.8a1 1 0 001-1L12 4" />
            </svg>
            <svg
              v-else-if="section.icon === 'archive'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M3 8.5l3.3 3.3L13 5" />
            </svg>
            <svg
              v-else-if="section.icon === 'clock'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <circle cx="8" cy="8" r="5.5" /><path d="M8 5.25v3.2L10.4 9.8" />
            </svg>
            <svg
              v-else-if="section.icon === 'aside'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M2.5 5.5h11v7.5h-11z" /><path d="M5.5 8.5h5" /><path d="M8 1.8v4" /><path d="M6.2 4L8 5.8 9.8 4" />
            </svg>
            <svg
              v-else-if="section.icon === 'spam'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M8 1.5l5 2.9v7.2L8 14.5l-5-2.9V4.4L8 1.5Z" /><path d="M8 5.1v3.5" /><circle cx="8" cy="11" r=".7" fill="currentColor" stroke="none" />
            </svg>
            <svg
              v-else-if="section.icon === 'bell'"
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <path d="M4.5 11.5h7l-1-1.5V7a2.5 2.5 0 10-5 0v3l-1 1.5Z" /><path d="M6.5 12.5a1.5 1.5 0 003 0" />
            </svg>
            <svg
              v-else
              width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
            >
              <circle cx="8" cy="8" r="5.5" /><path d="M4.5 11.5l7-7" />
            </svg>
          </template>
        </NavItem>
      </div>

    </div>

    <!-- Favorite folders -->
    <div v-if="mailboxes.hasVisibleCustomFolders" class="grid gap-1">
      <NavItem
        v-if="favoriteFolderGroups.length === 0"
        :label="$t('nav.folders.all')"
        @click="emit('openFoldersPanel')"
      >
        <template #icon>
          <AppIcon><path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h3l1.5 1.5h4.5A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z" /></AppIcon>
        </template>
        <template #trailing>
          <AppIcon><path d="m6 3.5 4.5 4.5L6 12.5" /></AppIcon>
        </template>
      </NavItem>
      <div v-else class="px-2.5 pt-1">
        <SectionHeading :label="$t('nav.folders.title')">
          <template #trailing>
            <AppButton

              @click="emit('openFoldersPanel')"
              variant="ghost"
              size="sm"
            >
              {{ $t('common.more') }}
            </AppButton>
          </template>
        </SectionHeading>
      </div>

      <div v-if="favoriteFolderGroups.length > 0" class="grid gap-[2px]">
        <div v-for="group in favoriteFolderGroups" :key="group.accountId" class="grid gap-[2px]">
          <p class="my-1 px-2.5 text-tertiary text-2xs font-semibold tracking-wide">{{ group.accountEmail }}</p>

          <div
            v-for="row in group.rows"
            :key="row.id"
            class="grid grid-cols-[16px_minmax(0,1fr)] items-center gap-1"
            :class="{ 'drop-target rounded-lg': dragHoverFolderKey === `${row.accountId}:${row.node.id}` }"
            :style="{ paddingLeft: `${row.depth * 16}px` }"
            @dragover.prevent="onDragOverFolder(`${row.accountId}:${row.node.id}`, $event)"
            @dragleave="onDragLeave(`${row.accountId}:${row.node.id}`)"
            @drop.prevent="onDropToFolder(row.node.folderId ?? '', $event)"
          >
            <IconButton
              v-if="row.node.children.length > 0"
              class="inline-flex items-center justify-center w-4 h-4 rounded-md bg-transparent text-tertiary cursor-pointer hover:bg-hover hover:text-primary transition-colors"
              :aria-label="$t('nav.toggleSection', { name: row.node.label })"
              @click="toggleOpen(`folder:${row.accountId}:${row.node.path}`, true)"
             :title="$t('nav.toggleSection', { name: row.node.label })">
              <svg
                :class="['transition-transform duration-100', isOpen(`folder:${row.accountId}:${row.node.path}`, true) ? 'rotate-90' : '']"
                width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
              >
                <path d="M4 2.5L8 6 4 9.5" />
              </svg>
            </IconButton>
            <span v-else class="inline-flex items-center justify-center w-4 h-4" />

            <NavItem
              :label="row.node.label"
              :active="activeNodeId === `folder:${row.accountId}:${row.node.path}`"
              :count="row.node.unreadCount > 0 ? row.node.unreadCount : undefined"
              @click="selectFavoriteFolder(row.accountId, row.node.path)"
            >
              <template #icon>
                <svg
                  width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"
                >
                  <path d="M2 4.5h4l1.2 1.5H14v6.5a1 1 0 01-1 1H3a1 1 0 01-1-1z" />
                </svg>
              </template>
            </NavItem>
          </div>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* Drop target highlight while dragging threads onto a folder */
.drop-target {
  outline: 2px solid var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
}

/* Fold transition */
.fold-enter-active,
.fold-leave-active {
  transition:
    opacity var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
  transform-origin: top left;
}

.fold-enter-from,
.fold-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}
</style>
