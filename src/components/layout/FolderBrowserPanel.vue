<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAccountsStore } from '@/stores/accounts';
import { useMailboxesStore, type FolderTreeNode } from '@/stores/mailboxes';
import { useThreadsStore } from '@/stores/threads';
import { useToast } from '@/composables/useToast';

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  close: [];
  navigate: [];
}>();

type TreeRow = {
  id: string;
  node: FolderTreeNode;
  depth: number;
  accountId: string;
};

const { t } = useI18n();
const router = useRouter();
const accounts = useAccountsStore();
const mailboxes = useMailboxesStore();
const threads = useThreadsStore();

const toast = useToast();
const panelRef = ref<HTMLElement | null>(null);
const busy = ref(false);
const creatingFor = ref<string | null>(null);
const newFolderName = ref('');
const renamingId = ref<string | null>(null);
const renameValue = ref('');
const canManageFolders = computed(() => mailboxes.folderManagementAvailable);

/** Folder types Satchel routes on; the backend refuses to touch them too. */
const PROTECTED = new Set(['inbox', 'sent', 'drafts', 'trash', 'archive', 'spam']);
function isProtected(node: FolderTreeNode): boolean {
  return PROTECTED.has((node.mailboxType ?? '').toLowerCase());
}

function startCreate(accountId: string) {
  creatingFor.value = accountId;
  newFolderName.value = '';
}

async function confirmCreate(accountId: string) {
  const name = newFolderName.value.trim();
  if (!name || busy.value) return;
  busy.value = true;
  try {
    await mailboxes.createFolder({ accountId, name });
    toast.success(t('nav.folderBrowser.created', { name }));
    creatingFor.value = null;
    newFolderName.value = '';
  } catch (err) {
    toast.error(err instanceof Error ? err.message : String(err));
  } finally {
    busy.value = false;
  }
}

function startRename(rowId: string, currentLabel: string) {
  renamingId.value = rowId;
  renameValue.value = currentLabel;
}

async function confirmRename(folderId: string) {
  const name = renameValue.value.trim();
  if (!name || busy.value) return;
  busy.value = true;
  try {
    await mailboxes.renameFolder({ folderId, newName: name });
    toast.success(t('nav.folderBrowser.renamed'));
    renamingId.value = null;
  } catch (err) {
    toast.error(err instanceof Error ? err.message : String(err));
  } finally {
    busy.value = false;
  }
}

async function removeFolder(folderId: string, label: string) {
  if (busy.value) return;
  if (!window.confirm(t('nav.folderBrowser.confirmDelete', { name: label }))) return;
  busy.value = true;
  try {
    await mailboxes.deleteFolder(folderId);
    toast.success(t('nav.folderBrowser.deleted', { name: label }));
  } catch (err) {
    toast.error(err instanceof Error ? err.message : String(err));
  } finally {
    busy.value = false;
  }
}
const openNodes = ref<Record<string, boolean>>({});

const accountRows = computed(() =>
  mailboxes.accountFolderTrees.map((group) => ({
    ...group,
    rows: flattenTree(group.tree, group.accountId),
  })),
);

function flattenTree(nodes: FolderTreeNode[], accountId: string, depth = 0): TreeRow[] {
  return nodes.flatMap((node) => {
    const row: TreeRow = {
      id: `browser:${accountId}:${node.path}`,
      node,
      depth,
      accountId,
    };

    if (!node.children.length || !isOpen(`${accountId}:${node.path}`, true)) {
      return [row];
    }

    return [row, ...flattenTree(node.children, accountId, depth + 1)];
  });
}

function isOpen(key: string, fallback = false): boolean {
  const current = openNodes.value[key];
  if (typeof current === 'boolean') {
    return current;
  }
  return fallback;
}

function toggleOpen(accountId: string, path: string) {
  const key = `${accountId}:${path}`;
  openNodes.value[key] = !isOpen(key, true);
}

function selectFolder(accountId: string, path: string) {
  emit('navigate');
  accounts.setActiveAccount(accountId);
  threads.workflowFilter = 'all';
  threads.setMailboxFilter({ kind: 'folder', value: path });
  void router.push({ name: 'inbox' });
  emit('close');
}

function activeFolderId(accountId: string, path: string): string {
  if (threads.mailboxFilter.kind !== 'folder' || threads.mailboxFilter.value !== path) {
    return '';
  }
  if (accounts.activeAccountId !== accountId) {
    return '';
  }
  return `browser:${accountId}:${path}`;
}

function onDocumentPointerDown(event: PointerEvent) {
  const target = event.target;
  if (!(target instanceof Node)) {
    return;
  }

  if (panelRef.value?.contains(target)) {
    return;
  }

  emit('close');
}

watch(
  () => props.open,
  (open) => {
    if (typeof document === 'undefined') {
      return;
    }

    if (open) {
      document.addEventListener('pointerdown', onDocumentPointerDown);
    } else {
      document.removeEventListener('pointerdown', onDocumentPointerDown);
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (typeof document !== 'undefined') {
    document.removeEventListener('pointerdown', onDocumentPointerDown);
  }
});
</script>

<template>
  <aside v-if="props.open" ref="panelRef" class="panel">
    <header class="panel-header">
      <div>
        <h3>{{ $t('nav.folderBrowser.title') }}</h3>
      </div>
      <IconButton class="close-btn" :aria-label="$t('nav.folderBrowser.close')" @click="emit('close')" :title="$t('nav.folderBrowser.close')">
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <path d="M3 3l10 10M13 3L3 13" />
        </svg>
      </IconButton>
    </header>

    <p class="panel-copy">
      {{ $t('nav.folderBrowser.hint') }}
    </p>

    <div class="panel-scroll">
      <section
        v-for="group in accountRows"
        :key="group.accountId"
        class="account-block"
      >
        <header class="account-header">
          <span class="account-email">{{ group.accountEmail }}</span>
          <span class="account-count">{{ group.rows.length }}</span>
          <AppButton
            v-if="canManageFolders"
            class="manage-btn"
            :title="$t('nav.folderBrowser.newFolder')"
            :disabled="busy"
            @click="startCreate(group.accountId)"
            variant="ghost"
            size="sm"
          >
            {{ $t('nav.folderBrowser.newFolderShort') }}
          </AppButton>
        </header>

        <form
          v-if="creatingFor === group.accountId"
          class="folder-form"
          @submit.prevent="void confirmCreate(group.accountId)"
        >
          <input
            v-model="newFolderName"
            class="folder-input"
            :placeholder="$t('nav.folderBrowser.namePlaceholder')"
            :aria-label="$t('nav.folderBrowser.nameLabel')"
            autofocus
            @keydown.esc="creatingFor = null"
          />
          <AppButton type="submit" class="manage-btn" :disabled="busy || !newFolderName.trim()" variant="primary" size="sm">{{ $t('common.create') }}</AppButton>
          <AppButton class="manage-btn ghost" @click="creatingFor = null" variant="ghost" size="sm">{{ $t('common.cancel') }}</AppButton>
        </form>

        <div class="tree-list">
          <template v-for="row in group.rows" :key="row.id">
          <div
            class="tree-row"
            :class="{ active: activeFolderId(row.accountId, row.node.path) === row.id }"
            :style="{ '--depth': row.depth }"
          >
            <IconButton
              v-if="row.node.children.length > 0"
              class="chevron-btn"
              :aria-label="$t('nav.toggleSection', { name: row.node.label })"
              @click="toggleOpen(row.accountId, row.node.path)"
             :title="$t('nav.toggleSection', { name: row.node.label })">
              <svg
                class="chevron"
                :class="{ open: isOpen(`${row.accountId}:${row.node.path}`, true) }"
                width="12"
                height="12"
                viewBox="0 0 12 12"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M4 2.5L8 6 4 9.5" />
              </svg>
            </IconButton>
            <span v-else class="chevron-spacer" />

            <CardButton class="folder-btn" @click="selectFolder(row.accountId, row.node.path)">
              <svg
                class="folder-icon"
                width="16"
                height="16"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <path d="M2 4.5h4l1.2 1.5H14v6.5a1 1 0 01-1 1H3a1 1 0 01-1-1z" />
              </svg>
              <span class="folder-label">{{ row.node.label }}</span>
              <span v-if="row.node.unreadCount > 0" class="folder-count">{{ row.node.unreadCount }}</span>
            </CardButton>

            <template v-if="canManageFolders && row.node.folderId && !isProtected(row.node)">
              <AppButton
                class="row-action"
                :title="$t('common.rename')"
                :disabled="busy"
                @click="startRename(row.id, row.node.label)"
                variant="ghost"
                size="sm"
              >
                {{ $t('common.rename') }}
              </AppButton>
              <AppButton
                class="row-action danger"
                :title="$t('common.delete')"
                :disabled="busy"
                @click="void removeFolder(row.node.folderId!, row.node.label)"
                variant="danger"
                size="sm"
              >
                {{ $t('common.delete') }}
              </AppButton>
            </template>

            <IconButton
              class="favorite-btn"
              :class="{ active: mailboxes.isFavoriteFolder(row.accountId, row.node.path) }"
              :title="mailboxes.isFavoriteFolder(row.accountId, row.node.path)
                ? $t('nav.folderBrowser.removeFavorite')
                : $t('nav.folderBrowser.addFavorite')"
              :aria-label="mailboxes.isFavoriteFolder(row.accountId, row.node.path)
                ? $t('nav.folderBrowser.removeFavorite')
                : $t('nav.folderBrowser.addFavorite')"
              @click="mailboxes.toggleFavoriteFolder(row.accountId, row.node.path)"
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <path d="M8 2.1l1.8 3.6 4 .6-2.9 2.8.7 4L8 11.2 4.4 13l.7-4L2.2 6.3l4-.6L8 2.1Z" />
              </svg>
            </IconButton>
          </div>

          <form
            v-if="renamingId === row.id && row.node.folderId"
            class="folder-form"
            @submit.prevent="void confirmRename(row.node.folderId!)"
          >
            <input
              v-model="renameValue"
              class="folder-input"
              :aria-label="$t('nav.folderBrowser.renameLabel')"
              autofocus
              @keydown.esc="renamingId = null"
            />
            <AppButton type="submit" class="manage-btn" :disabled="busy || !renameValue.trim()" variant="primary" size="sm">{{ $t('common.save') }}</AppButton>
            <AppButton class="manage-btn ghost" @click="renamingId = null" variant="ghost" size="sm">{{ $t('common.cancel') }}</AppButton>
          </form>
          </template>
        </div>
      </section>
    </div>
  </aside>
</template>

<style scoped>

.folder-form {
  display: flex;
  gap: 6px;
  align-items: center;
  padding: 6px 4px 8px;
}

.folder-input {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-xs);
  background: var(--bg-raised);
  padding: 5px 8px;
  font: inherit;
  font-size: var(--font-size-xs);
}

.folder-input:focus { border-color: var(--accent-primary); }

.panel {
  position: absolute;
  top: 10px;
  left: calc(100% + 12px);
  bottom: 10px;
  width: min(380px, 42vw);
  display: grid;
  grid-template-rows: auto auto 1fr;
  gap: 10px;
  padding: 18px 16px 14px;
  border: var(--border-default);
  border-radius: var(--radius-lg);
  background: var(--bg-overlay);
  box-shadow: var(--shadow-lg);
  backdrop-filter: blur(20px);
  z-index: 30;
}

.panel-header {
  display: flex;
  align-items: start;
  justify-content: space-between;
  gap: 12px;
}

h3 {
  margin: 0;
  font-size: var(--font-size-2xl);
  letter-spacing: -0.03em;
}

.close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  height: 34px;
  border: 0;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--text-primary) 6%, transparent);
  color: var(--text-secondary);
  cursor: pointer;
}

.close-btn:hover {
  background: color-mix(in srgb, var(--text-primary) 12%, transparent);
  color: var(--text-primary);
}

.panel-copy {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.45;
}

.panel-scroll {
  min-height: 0;
  overflow: auto;
  display: grid;
  gap: 16px;
  padding-right: 2px;
  align-content: start;
}

.account-block {
  display: grid;
  gap: 8px;
  align-content: start;
}

.account-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-top: 4px;
}

.account-email {
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
  color: var(--text-primary);
}

.account-count {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.tree-list {
  display: grid;
  gap: 2px;
  align-content: start;
}

.tree-row {
  --depth-step: 18px;
  display: grid;
  grid-template-columns: 16px minmax(0, 1fr) 28px;
  align-items: center;
  gap: 8px;
  padding-left: calc(var(--depth) * var(--depth-step));
  border-radius: var(--radius-md);
}

.tree-row.active {
  background: var(--bg-active);
}

.chevron-btn,
.favorite-btn,
.folder-btn {
  border: 0;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.chevron-btn,
.chevron-spacer,
.favorite-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
}

.chevron {
  transition: transform var(--duration-fast) var(--ease-out);
}

.chevron.open {
  transform: rotate(90deg);
}

.folder-btn {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: 10px;
  min-width: 0;
  padding: 8px 0;
  text-align: left;
}

.folder-btn:hover {
  color: var(--text-primary);
}

.folder-icon {
  color: var(--text-tertiary);
}

.folder-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--font-size-sm);
}

.folder-count {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
}

.favorite-btn {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-sm);
  color: var(--text-tertiary);
}

.favorite-btn:hover {
  background: color-mix(in srgb, var(--text-primary) 6%, transparent);
  color: var(--accent-yellow);
}

.favorite-btn.active {
  color: var(--accent-yellow);
}
</style>
