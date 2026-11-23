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
import { labsSidebarItems } from '@/labs/state';
import { useThreadActions } from '@/composables/useThreadActions';
import { matchesWorkflowFilter, type WorkflowFilter } from '@/types/workflow';
import { MAILBOX_SECTIONS, type CounterMode, type SectionIcon, type SectionSpec } from '@/config/mailboxSections';
import { showMailList } from '@/router/mailViews';
import NavItem from '@/components/ui/NavItem.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { Ban, Bell, Bookmark, Check, ChevronRight, Clock, FilePen, Folder, Inbox, Send, SendHorizontal, ShieldAlert, Trash2 } from 'lucide';
import type { IconNode } from 'lucide';
import SectionHeading from '@/components/ui/SectionHeading.vue';
import { outboxVisibleCount } from '@/utils/outboxLabels';
import { activeMailboxNavNode } from '@/utils/mailboxNav';
import { draggedThreadIds, isThreadDrag } from '@/utils/appDrag';

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

const SECTION_ICONS: Record<SectionIcon, IconNode> = {
  inbox: Inbox,
  drafts: FilePen,
  sent: Send,
  outbox: SendHorizontal,
  trash: Trash2,
  // The section is Done; it shows the archive folder.
  archive: Check,
  clock: Clock,
  aside: Bookmark,
  spam: ShieldAlert,
  bell: Bell,
  blocked: Ban,
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
      // Rows stay put across accounts; their numbers follow the chosen scope.
      const total = sectionCount(section, accounts.activeAccountId);
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
            <AppIcon :icon="SECTION_ICONS[section.icon]" />
          </template>
        </NavItem>
      </div>
      <!-- Rows of switched-on Labs experiments (Chats), right under the Inbox. -->
      <template v-if="section.id === 'inbox'">
        <component :is="item" v-for="(item, index) in labsSidebarItems" :key="index" @navigate="emit('navigate')" />
      </template>

    </div>

    <!-- Favorite folders -->
    <div v-if="mailboxes.hasVisibleCustomFolders" class="grid gap-1">
      <NavItem
        v-if="favoriteFolderGroups.length === 0"
        :label="$t('nav.folders.all')"
        @click="emit('openFoldersPanel')"
      >
        <template #icon>
          <AppIcon :icon="Folder" />
        </template>
        <template #trailing>
          <AppIcon :icon="ChevronRight" />
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
              <AppIcon :class="['transition-transform duration-100', isOpen(`folder:${row.accountId}:${row.node.path}`, true) ? 'rotate-90' : '']" :icon="ChevronRight" size="sm" />
            </IconButton>
            <span v-else class="inline-flex items-center justify-center w-4 h-4" />

            <NavItem
              :label="row.node.label"
              :active="activeNodeId === `folder:${row.accountId}:${row.node.path}`"
              :count="row.node.unreadCount > 0 ? row.node.unreadCount : undefined"
              @click="selectFavoriteFolder(row.accountId, row.node.path)"
            >
              <template #icon>
                <AppIcon :icon="Folder" />
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
