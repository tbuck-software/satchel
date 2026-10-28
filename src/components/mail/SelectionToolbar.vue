<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { useThreadsStore } from '@/stores/threads';
import { useUiStore } from '@/stores/ui';
import { useTopicsStore } from '@/stores/topics';
import { useThreadActions } from '@/composables/useThreadActions';
import { useToast } from '@/composables/useToast';
import type { InboxSenderBucket } from '@/stores/settings';
import type { ThreadCategory } from '@/types/mail';
import { useShortcutHints } from '@/composables/useShortcutHints';
import type { AnchorBox } from '@/utils/popoverPlacement';

const ContextMenu = defineAsyncComponent(() => import('@/components/ui/ContextMenu.vue'));
const MenuItem = defineAsyncComponent(() => import('@/components/ui/MenuItem.vue'));
const props = defineProps<{ busy: boolean }>();
const { t } = useI18n();
const { hint } = useShortcutHints();
const threads = useThreadsStore();
const topics = useTopicsStore();
const ui = useUiStore();
const actions = useThreadActions();
const toast = useToast();
const selectedCategory = ref<ThreadCategory>('updates');
const rememberSelectedSenders = ref(false);
const classificationBusy = ref(false);
const busy = computed(() => props.busy || classificationBusy.value);
const ids = () => [...threads.selectedThreadIds];
const anyUnread = computed(() => threads.selectedThreads.some(thread => thread.unreadCount > 0));
const allPinned = computed(() => threads.selectedThreads.every(thread => thread.isStarred));
const menuOpen = ref(false);
const menuAnchor = ref<AnchorBox | null>(null);

const classificationOptions = computed<Array<{
  label: string;
  category: ThreadCategory;
  senderBucket: InboxSenderBucket;
}>>(() => [
  { label: t('thread.bucket.personal'), category: 'personal', senderBucket: 'personal' },
  { label: t('thread.bucket.notifications'), category: 'updates', senderBucket: 'notifications' },
  { label: t('thread.bucket.newsletter'), category: 'promotions', senderBucket: 'newsletter' },
]);

async function classifySelected() {
  const option = classificationOptions.value.find((entry) => entry.category === selectedCategory.value);
  if (!option || threads.selectedThreadIds.length === 0 || classificationBusy.value) {
    return;
  }

  const selectedThreads = threads.selectedThreads;
  classificationBusy.value = true;
  const applied = await actions.attempt(async () => {
    // Wait for every write, including failures, before releasing the selection.
    const outcomes = await Promise.allSettled(
      selectedThreads.map((thread) => rememberSelectedSenders.value
        ? threads.setSenderCategory(thread.id, option.category, option.senderBucket)
        : threads.setThreadCategory(thread.id, option.category)),
    );
    const failed = outcomes.find((outcome) => outcome.status === 'rejected');
    if (failed?.status === 'rejected') throw failed.reason;
  });
  classificationBusy.value = false;
  if (!applied) return;
  threads.clearSelection();
  toast.success(t('inbox.classifiedToast', selectedThreads.length));
}

function openMenu(event: MouseEvent) {
  menuAnchor.value = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menuOpen.value = true;
}

/** File…: one picker for folders and topics, the same one `v` opens. */
function openFile() {
  topics.openPicker({ itemType: 'thread', itemIds: ids() });
}

function run(action: 'pin' | 'aside' | 'waiting' | 'spam') {
  menuOpen.value = false;
  const target = ids();
  if (action === 'pin') void actions.setPinned(target, !allPinned.value);
  if (action === 'aside') void actions.setAside(target);
  if (action === 'waiting') void actions.waitForReply(target);
  if (action === 'spam') void actions.markSpam(target);
}
</script>

<template>
  <div class="selection-tools no-drag" role="group" :aria-label="t('thread.selection.label', { count: threads.selectedThreadIds.length })">
    <span class="selection-count">{{ t('thread.selection.label', { count: threads.selectedThreadIds.length }) }}</span>
    <!-- Done and Snooze carry words; the rest are the toolbar's icons, so the
         row fits the list column at every width instead of wrapping. -->
    <div class="selection-actions">
      <AppButton variant="primary" size="sm" :disabled="busy" :title="t('thread.selection.done', { count: threads.selectedThreadIds.length })" :shortcut="hint('markDone')" @click="void actions.archive(ids())">
        <template #icon><AppIcon><path d="M3 8.5 6.2 11.7 13 5" /></AppIcon></template>
        {{ t('thread.selection.done', { count: threads.selectedThreadIds.length }) }}
      </AppButton>
      <AppButton variant="ghost" size="sm" :disabled="busy" :title="t('thread.actions.snooze')" :shortcut="hint('moveSnoozed')" @click="ui.openReminder(ids())">
        <template #icon><AppIcon><circle cx="8" cy="8" r="6" /><path d="M8 4v4l2.5 1.5" /></AppIcon></template>
        {{ t('thread.actions.snooze') }}
      </AppButton>
      <IconButton :title="t('common.delete')" :disabled="busy" :shortcut="hint('trash')" @click="void actions.trash(ids())">
        <AppIcon><path d="M2 4h12M6 4V2h4v2M4 4l1 10h6l1-10" /></AppIcon>
      </IconButton>
      <IconButton :title="anyUnread ? t('thread.actions.markRead') : t('thread.actions.markUnread')" :disabled="busy" :shortcut="hint('toggleRead')" @click="void actions.setRead(ids(), anyUnread, { undo: true })">
        <AppIcon><rect x="2" y="3" width="12" height="10" rx="2" /><path d="m2 5 6 4 6-4" /></AppIcon>
      </IconButton>
      <IconButton :title="t('thread.actions.fileTo')" :disabled="busy" :shortcut="hint('fileTo')" @click="openFile">
        <AppIcon><path d="M2 4h5l2 2h5v7H2V4Z" /></AppIcon>
      </IconButton>
      <IconButton :title="t('thread.toolbar.moreActions')" :disabled="busy" aria-haspopup="menu" :aria-expanded="menuOpen" @click="openMenu">
        <AppIcon><circle cx="3" cy="8" r=".75" fill="currentColor" /><circle cx="8" cy="8" r=".75" fill="currentColor" /><circle cx="13" cy="8" r=".75" fill="currentColor" /></AppIcon>
      </IconButton>
    </div>
    <ContextMenu v-if="menuOpen" :open="menuOpen" :anchor="menuAnchor" :label="t('thread.toolbar.moreActions')" :width="280" @close="menuOpen = false">
      <MenuItem :icon="allPinned ? 'unpin' : 'pin'" :label="allPinned ? t('thread.actions.unpin') : t('thread.actions.pin')" @select="run('pin')" />
      <MenuItem icon="aside" :label="t('thread.actions.setAside')" @select="run('aside')" />
      <MenuItem icon="clock" :label="t('thread.state.waiting')" @select="run('waiting')" />
      <MenuItem :label="t('thread.toolbar.markSpam')" tone="danger" @select="run('spam')">
        <template #icon><AppIcon><path d="M2 4h12v10H2V4ZM5 2h6M6 7l4 4m0-4-4 4" /></AppIcon></template>
      </MenuItem>
      <div class="selection-classification">
        <select v-model="selectedCategory" class="selection-select" :aria-label="$t('inbox.classifyAs')">
          <option
            v-for="option in classificationOptions"
            :key="option.category"
            :value="option.category"
          >
            {{ option.label }}
          </option>
        </select>
        <label class="remember-senders">
          <input v-model="rememberSelectedSenders" type="checkbox" />
          <span>{{ $t('inbox.rememberSender') }}</span>
        </label>
        <AppButton variant="soft" size="sm" :disabled="busy" @click="void classifySelected()">
          {{ $t('inbox.classify') }}
        </AppButton>
      </div>
    </ContextMenu>
  </div>
</template>

<style scoped>
.selection-tools {
  min-width: 0;
  padding: var(--space-sm) var(--space-md);
  border-bottom: 1px solid var(--border-color-muted);
  background: var(--bg-list);
}
.selection-count {
  display: block;
  margin-bottom: var(--space-xs);
  color: var(--text-primary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
}
.selection-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-xs);
}
.selection-classification {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-sm);
  padding: var(--space-sm);
  border-top: 1px solid var(--border-color-muted);
}
.selection-select {
  flex: 0 0 auto;
  min-width: 0;
  height: var(--space-2xl);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-primary);
  padding: 0 var(--space-sm);
  font-size: var(--font-size-xs);
}

.remember-senders {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: var(--space-1-5);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  white-space: nowrap;
}

.remember-senders input {
  width: var(--space-3-5);
  height: var(--space-3-5);
  margin: 0;
  accent-color: var(--accent-primary);
}

@media (max-width: 600px) {
  .selection-actions :deep(.app-button) {
    min-height: var(--touch-target);
    height: auto;
    white-space: normal;
  }
}
</style>
