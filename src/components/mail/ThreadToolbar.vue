<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  ALargeSmall, Ban, Bell, BellOff, Bookmark, BrushCleaning, Check, ChevronDown, ChevronLeft,
  ChevronUp, Clock, Command, Copy, Download, Ellipsis, FolderInput, Forward, Globe, Inbox, Link,
  ListTodo, Mail, MailOpen, MessageSquare, Minus, Pin, PinOff, Plus, Printer, Reply, RotateCcw,
  Send, Share, ShieldAlert, Trash2, User, UserPlus,
} from 'lucide';
import IconButton from '@/components/ui/IconButton.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import PluginSlotHost from '@/components/plugins/PluginSlotHost.vue';
import { useShortcutHints } from '@/composables/useShortcutHints';
import type { AnchorBox } from '@/utils/popoverPlacement';

type Submenu = 'block' | 'organize' | 'print' | 'markdown' | 'export';
type MessageScope = 'latest' | 'conversation';
type ExportFormat = 'eml' | 'markdown' | 'text';

const props = withDefaults(
  defineProps<{
    threadId: string;
    subject: string;
    selectionCount?: number;
    isUnread?: boolean;
    isStarred?: boolean;
    isAside?: boolean;
    /** Done already: the primary action turns into its inverse. */
    isDone?: boolean;
    isMuted?: boolean;
    /** The filter marked it, or it sits in Spam: the menu offers Not spam. */
    canMarkNotSpam?: boolean;
    canDelete?: boolean;
    showBack?: boolean;
    senderAddress?: string;
    canReply?: boolean;
    /**
     * The reader offers the reply itself: a reply foot, or the card of an
     * unsent draft. Reply then lives there and in the menu; a second button
     * up here would only compete with it.
     */
    replyInReader?: boolean;
    /** Number of downloadable attachments across the thread. */
    attachmentCount?: number;
    readerZoom?: number;
    /** Where this thread sits in the list behind it, 1-based. */
    position?: number;
    total?: number;
  }>(),
  {
    selectionCount: 0,
    isUnread: false,
    isStarred: false,
    isAside: false,
    isDone: false,
    isMuted: false,
    canMarkNotSpam: false,
    canDelete: true,
    showBack: false,
    senderAddress: '',
    canReply: true,
    replyInReader: false,
    attachmentCount: 0,
    readerZoom: 100,
    position: 0,
    total: 0,
  },
);

const emit = defineEmits<{
  back: [];
  previous: [];
  next: [];
  archive: [];
  moveToInbox: [];
  toggleRead: [];
  togglePin: [];
  setAside: [];
  snooze: [];
  delete: [];
  toggleMuted: [];
  spam: [];
  notSpam: [];
  block: [scope: 'address' | 'domain'];
  sweep: [];
  addTask: [];
  print: [scope: MessageScope];
  copyMarkdown: [scope: MessageScope];
  copyLink: [];
  export: [format: ExportFormat];
  saveAttachments: [];
  /** Delta in percentage points; 0 resets to 100. */
  zoom: [delta: number];
  reply: [];
  resend: [];
  forward: [];
  allActions: [];
  organize: [kind: 'assignee' | 'sharing'];
  fileTo: [];
}>();

const { t } = useI18n();
const { hint } = useShortcutHints();

const menuOpen = ref(false);
const menuAnchor = ref<AnchorBox | null>(null);
const activeSubmenu = ref<Submenu | null>(null);
const submenuAnchor = ref<AnchorBox | null>(null);
const submenuFocusFirst = ref(false);
const moreRef = ref<HTMLElement | null>(null);

const SUBMENU_LABELS: Record<Submenu, string> = {
  block: 'thread.actions.blockSender',
  organize: 'thread.toolbar.organize',
  print: 'thread.toolbar.print',
  markdown: 'thread.toolbar.copyMarkdown',
  export: 'thread.toolbar.exportTo',
};
const submenuLabel = computed(() => (activeSubmenu.value ? t(SUBMENU_LABELS[activeSubmenu.value]) : ''));

function targetLabel(action: string): string {
  return props.selectionCount ? t('thread.selection.action', { action, count: props.selectionCount }) : action;
}

function openMenu() {
  const rect = moreRef.value?.getBoundingClientRect();
  if (!rect) return;
  // A zero-width point under the trigger's right edge: the menu opens down and
  // to the left, flush with the button.
  menuAnchor.value = { left: rect.right, right: rect.right, top: rect.bottom + 8, bottom: rect.bottom + 8 };
  menuOpen.value = true;
}

function closeSubmenu() {
  activeSubmenu.value = null;
  submenuAnchor.value = null;
}

function closeMenu() {
  closeSubmenu();
  menuOpen.value = false;
}

function toggleMenu() {
  if (menuOpen.value) closeMenu();
  else openMenu();
}

/** Hover opens a flyout in place; Enter, Space or → also moves focus into it. */
function openSubmenu(submenu: Submenu, event: Event) {
  const trigger = (event.currentTarget as HTMLElement | null)?.getBoundingClientRect();
  if (!trigger) return;
  submenuAnchor.value = { left: trigger.left, top: trigger.top - 7, right: trigger.right, bottom: trigger.bottom };
  submenuFocusFirst.value = event.type === 'click' && (event as MouseEvent).detail === 0;
  activeSubmenu.value = submenu;
}

function run(action: 'toggleRead' | 'togglePin' | 'setAside' | 'forward' | 'toggleMuted' | 'spam' | 'notSpam' | 'reply' | 'resend' | 'allActions' | 'copyLink' | 'fileTo') {
  closeMenu();
  if (action === 'toggleRead') emit('toggleRead');
  if (action === 'togglePin') emit('togglePin');
  if (action === 'setAside') emit('setAside');
  if (action === 'forward') emit('forward');
  if (action === 'toggleMuted') emit('toggleMuted');
  if (action === 'spam') emit('spam');
  if (action === 'notSpam') emit('notSpam');
  if (action === 'reply') emit('reply');
  if (action === 'resend') emit('resend');
  if (action === 'allActions') emit('allActions');
  if (action === 'copyLink') emit('copyLink');
  if (action === 'fileTo') emit('fileTo');
}

function runSweep() {
  closeMenu();
  emit('sweep');
}

function runAddTask() {
  closeMenu();
  emit('addTask');
}

function runBlock(scope: 'address' | 'domain') {
  closeMenu();
  emit('block', scope);
}

function runOrganize(kind: 'assignee' | 'sharing') {
  closeMenu();
  emit('organize', kind);
}

function runPrint(scope: MessageScope) {
  closeMenu();
  emit('print', scope);
}

function runCopyMarkdown(scope: MessageScope) {
  closeMenu();
  emit('copyMarkdown', scope);
}

function runSaveAttachments() {
  closeMenu();
  emit('saveAttachments');
}

function runExport(format: ExportFormat) {
  closeMenu();
  emit('export', format);
}
</script>

<template>
  <div class="thread-toolbar" data-tauri-drag-region>
    <div class="thread-toolbar__group">
      <template v-if="showBack">
        <IconButton size="lg" :title="$t('common.back')" :shortcut="hint('leaveItem')" @click="emit('back')">
          <AppIcon :icon="ChevronLeft" size="lg" />
        </IconButton>
        <span class="divider" />
      </template>

      <!-- The one thing this toolbar is for. Everything beside it is an icon,
           so the accent says which key you were reaching for. -->
      <!-- On a thread that is done already, "Done" would do nothing; the
           button offers the way back instead. -->
      <AppButton
        v-if="isDone"
        variant="primary"
        class="thread-toolbar__done"
        :title="targetLabel($t('commands.moveToInbox'))"
        :shortcut="hint('moveInbox')"
        @click="emit('moveToInbox')"
      >
        <template #icon>
          <AppIcon :icon="Inbox" size="lg" />
        </template>
        {{ selectionCount ? $t('thread.selection.toInbox', { count: selectionCount }) : $t('commands.moveToInbox') }}
      </AppButton>
      <AppButton
        v-else
        variant="primary"
        class="thread-toolbar__done"
        :title="targetLabel($t('common.done'))"
        :shortcut="hint('markDone')"
        @click="emit('archive')"
      >
        <template #icon>
          <AppIcon :icon="Check" size="lg" />
        </template>
        {{ selectionCount ? $t('thread.selection.done', { count: selectionCount }) : $t('common.done') }}
      </AppButton>
      <IconButton size="lg" :title="targetLabel($t('thread.toolbar.setReminder'))" :shortcut="hint('moveSnoozed')" @click="emit('snooze')">
        <AppIcon :icon="Clock" size="lg" />
      </IconButton>
      <IconButton v-if="canDelete" size="lg" tone="danger" :title="targetLabel($t('common.delete'))" :shortcut="hint('trash')" @click="emit('delete')">
        <AppIcon :icon="Trash2" size="lg" />
      </IconButton>
    </div>

    <div class="thread-toolbar__group">
      <!-- Where you are in the list, and the two steps either way. The
           keyboard has had this since the beginning; the pointer had not. -->
      <template v-if="total > 1 && position > 0">
        <span class="thread-toolbar__position">{{ $t('thread.toolbar.position', { position, total }) }}</span>
        <IconButton
          size="lg"
          :disabled="position <= 1"
          :title="$t('thread.toolbar.previous')"
          :shortcut="hint('prevThread')"
          @click="emit('previous')"
        >
          <AppIcon :icon="ChevronUp" size="lg" />
        </IconButton>
        <IconButton
          size="lg"
          :disabled="position >= total"
          :title="$t('thread.toolbar.next')"
          :shortcut="hint('nextThread')"
          @click="emit('next')"
        >
          <AppIcon :icon="ChevronDown" size="lg" />
        </IconButton>
      </template>

      <PluginSlotHost slot-name="top_toolbar" :context="{ threadId, subject }" />
      <AppButton v-if="canReply && !replyInReader" variant="soft" :title="$t('common.reply')" :shortcut="hint('reply')" @click="emit('reply')">
        <template #icon><AppIcon :icon="Reply" size="lg" /></template>
        {{ $t('common.reply') }}
      </AppButton>

      <span ref="moreRef" class="more-wrap">
        <IconButton
          size="lg"
          :title="$t('thread.toolbar.moreActions')"
          aria-haspopup="menu"
          :aria-expanded="menuOpen"
          @click="toggleMenu"
        >
          <AppIcon :icon="Ellipsis" size="lg" />
        </IconButton>
      </span>
    </div>
  </div>

  <ContextMenu
    :open="menuOpen"
    :anchor="menuAnchor"
    :label="$t('thread.toolbar.moreActions')"
    :width="312"
    @close="closeMenu"
  >
    <div v-if="canReply" class="menu-group" @mouseenter="closeSubmenu">
      <MenuItem :label="$t('common.reply')" :aria-keyshortcuts="hint('reply')?.aria" @select="run('reply')">
        <template #icon><AppIcon :icon="Reply" /></template>
        <template v-if="hint('reply')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('reply')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.resend')" @select="run('resend')">
        <template #icon><AppIcon :icon="Send" /></template>
      </MenuItem>
      <MenuItem :label="$t('common.forward')" :aria-keyshortcuts="hint('forward')?.aria" @select="run('forward')">
        <template #icon><AppIcon :icon="Forward" /></template>
        <template v-if="hint('forward')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('forward')?.label }}</kbd></template>
      </MenuItem>
    </div>

    <div class="menu-group">
      <MenuItem
        :label="targetLabel($t('thread.actions.fileTo'))"
        :aria-keyshortcuts="hint('fileTo')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('fileTo')"
      >
        <template #icon><AppIcon :icon="FolderInput" /></template>
        <template v-if="hint('fileTo')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('fileTo')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="targetLabel(isUnread ? $t('thread.actions.markRead') : $t('thread.actions.markUnread'))"
        :aria-keyshortcuts="hint('toggleRead')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('toggleRead')"
      >
        <template #icon><AppIcon :icon="isUnread ? MailOpen : Mail" /></template>
        <template v-if="hint('toggleRead')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('toggleRead')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="targetLabel(isStarred ? $t('thread.actions.unpin') : $t('thread.actions.pin'))"
        :aria-keyshortcuts="hint('pin')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('togglePin')"
      >
        <template #icon><AppIcon :icon="isStarred ? PinOff : Pin" /></template>
        <template v-if="hint('pin')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('pin')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="targetLabel(isAside ? $t('thread.actions.moveToInbox') : $t('thread.actions.setAside'))"
        :aria-keyshortcuts="hint(isAside ? 'moveInbox' : 'setAside')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('setAside')"
      >
        <template #icon><AppIcon :icon="isAside ? Inbox : Bookmark" /></template>
        <template v-if="hint(isAside ? 'moveInbox' : 'setAside')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint(isAside ? 'moveInbox' : 'setAside')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="targetLabel(isMuted ? $t('thread.actions.unmute') : $t('thread.actions.mute'))"
        :aria-keyshortcuts="hint('toggleMute')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('toggleMuted')"
      >
        <template #icon><AppIcon :icon="isMuted ? Bell : BellOff" /></template>
        <template v-if="hint('toggleMute')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('toggleMute')?.label }}</kbd></template>
      </MenuItem>
    </div>

    <div class="menu-group">
      <MenuItem
        :label="$t('sweep.menuItem')"
        :aria-keyshortcuts="hint('sweepSender')?.aria"
        @mouseenter="closeSubmenu"
        @select="runSweep"
      >
        <template #icon><AppIcon :icon="BrushCleaning" /></template>
        <template v-if="hint('sweepSender')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('sweepSender')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.actions.addTask')"
        :aria-keyshortcuts="hint('createTask')?.aria"
        @mouseenter="closeSubmenu"
        @select="runAddTask"
      >
        <template #icon><AppIcon :icon="ListTodo" /></template>
        <template v-if="hint('createTask')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('createTask')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="$t('common.block')"
        submenu
        :expanded="activeSubmenu === 'block'"
        @mouseenter="openSubmenu('block', $event)"
        @select="openSubmenu('block', $event)"
      >
        <template #icon><AppIcon :icon="Ban" /></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.organize')"
        submenu
        :expanded="activeSubmenu === 'organize'"
        @mouseenter="openSubmenu('organize', $event)"
        @select="openSubmenu('organize', $event)"
      >
        <template #icon><AppIcon :icon="UserPlus" /></template>
      </MenuItem>
      <MenuItem
        v-if="canMarkNotSpam"
        :label="targetLabel($t('thread.actions.notSpam'))"
        :aria-keyshortcuts="hint('markNotSpam')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('notSpam')"
      >
        <template #icon><AppIcon :icon="Inbox" /></template>
        <template v-if="hint('markNotSpam')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('markNotSpam')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="targetLabel($t('thread.toolbar.markSpam'))"
        tone="danger"
        :aria-keyshortcuts="hint('markSpam')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('spam')"
      >
        <template #icon><AppIcon :icon="ShieldAlert" /></template>
        <template v-if="hint('markSpam')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('markSpam')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.print')"
        submenu
        :expanded="activeSubmenu === 'print'"
        @mouseenter="openSubmenu('print', $event)"
        @select="openSubmenu('print', $event)"
      >
        <template #icon><AppIcon :icon="Printer" /></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.link.copy')"
        :aria-keyshortcuts="hint('copyLink')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('copyLink')"
      >
        <template #icon><AppIcon :icon="Link" /></template>
        <template v-if="hint('copyLink')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('copyLink')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.copyMarkdown')"
        submenu
        :expanded="activeSubmenu === 'markdown'"
        @mouseenter="openSubmenu('markdown', $event)"
        @select="openSubmenu('markdown', $event)"
      >
        <template #icon><AppIcon :icon="Copy" /></template>
      </MenuItem>
    </div>

    <div class="menu-group" @mouseenter="closeSubmenu">
      <div class="zoom-row" role="group" :aria-label="$t('thread.toolbar.textSize')">
        <AppIcon :icon="ALargeSmall" class="zoom-row__icon" />
        <span>{{ $t('thread.toolbar.textSize') }}</span>
        <span class="zoom-controls">
          <IconButton
            class="zoom-btn"
            role="menuitem"
            :title="$t('thread.toolbar.smaller')"
            :aria-label="$t('thread.toolbar.decreaseText')"
            @click.stop="emit('zoom', -10)"
          ><AppIcon :icon="Minus" /></IconButton>
          <span class="zoom-value">{{ props.readerZoom }}%</span>
          <IconButton
            class="zoom-btn"
            role="menuitem"
            :title="$t('thread.toolbar.larger')"
            :aria-label="$t('thread.toolbar.increaseText')"
            @click.stop="emit('zoom', 10)"
          ><AppIcon :icon="Plus" /></IconButton>
          <IconButton
            class="zoom-btn"
            role="menuitem"
            :title="$t('common.reset')"
            :aria-label="$t('thread.toolbar.resetTextSize')"
            @click.stop="emit('zoom', 0)"
          ><AppIcon :icon="RotateCcw" /></IconButton>
        </span>
      </div>
    </div>

    <div class="menu-group">
      <MenuItem
        v-if="props.attachmentCount > 0"
        :label="$t('thread.toolbar.saveAllAttachments', { count: props.attachmentCount })"
        @mouseenter="closeSubmenu"
        @select="runSaveAttachments"
      >
        <template #icon><AppIcon :icon="Download" /></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.exportTo')"
        submenu
        :expanded="activeSubmenu === 'export'"
        @mouseenter="openSubmenu('export', $event)"
        @select="openSubmenu('export', $event)"
      >
        <template #icon><AppIcon :icon="Share" /></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.allActions')" :aria-keyshortcuts="hint('openCommandPalette')?.aria" @mouseenter="closeSubmenu" @select="run('allActions')">
        <template #icon><AppIcon :icon="Command" /></template>
        <template v-if="hint('openCommandPalette')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('openCommandPalette')?.label }}</kbd></template>
      </MenuItem>
    </div>
  </ContextMenu>

  <ContextMenu
    :open="menuOpen && activeSubmenu !== null"
    :anchor="submenuAnchor"
    :label="submenuLabel"
    :width="268"
    nested
    :focus-first="submenuFocusFirst"
    @close="closeSubmenu"
  >
    <template v-if="activeSubmenu === 'organize'">
      <MenuItem :label="$t('thread.toolbar.changeAssignee')" @select="runOrganize('assignee')" />
      <MenuItem :label="$t('thread.toolbar.changeSharing')" @select="runOrganize('sharing')" />
    </template>

    <template v-else-if="activeSubmenu === 'block'">
      <p class="submenu-title">{{ $t('thread.actions.blockSender') }}</p>
      <MenuItem :label="$t('thread.toolbar.blockAddress')" @select="runBlock('address')">
        <template #icon><AppIcon :icon="User" /></template>
        <span class="stacked-label"><strong>{{ $t('thread.toolbar.blockAddress') }}</strong><small>{{ senderAddress || $t('thread.toolbar.currentSender') }}</small></span>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.blockDomain')" @select="runBlock('domain')">
        <template #icon><AppIcon :icon="Globe" /></template>
      </MenuItem>
    </template>

    <template v-else-if="activeSubmenu === 'print'">
      <MenuItem :label="$t('thread.toolbar.latestMessageOnly')" @select="runPrint('latest')">
        <template #icon><AppIcon :icon="Mail" /></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.wholeConversation')" :aria-keyshortcuts="hint('print')?.aria" @select="runPrint('conversation')">
        <template #icon><AppIcon :icon="MessageSquare" /></template>
        <template v-if="hint('print')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('print')?.label }}</kbd></template>
      </MenuItem>
    </template>

    <template v-else-if="activeSubmenu === 'markdown'">
      <MenuItem :label="$t('thread.toolbar.copyLatest')" @select="runCopyMarkdown('latest')">
        <template #icon><AppIcon :icon="Mail" /></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.copyConversation')" @select="runCopyMarkdown('conversation')">
        <template #icon><AppIcon :icon="MessageSquare" /></template>
      </MenuItem>
    </template>

    <template v-else-if="activeSubmenu === 'export'">
      <MenuItem :label="$t('thread.toolbar.latestMessage')" @select="runExport('eml')">
        <template #icon><span class="format-icon">EML</span></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.conversationMarkdown')" @select="runExport('markdown')">
        <template #icon><span class="format-icon">MD</span></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.conversationText')" @select="runExport('text')">
        <template #icon><span class="format-icon">TXT</span></template>
      </MenuItem>
    </template>
  </ContextMenu>
</template>

<style scoped>
.zoom-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  cursor: default;
}

.zoom-row__icon {
  color: var(--text-tertiary);
}

.zoom-controls {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  margin-left: auto;
}

.zoom-btn {
  border: 1px solid var(--border-color);
  border-radius: var(--radius-xs);
  background: transparent;
  width: 22px;
  height: 22px;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-sm);
  line-height: 1;
  cursor: pointer;
}

.zoom-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.zoom-value {
  min-width: 40px;
  text-align: center;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-variant-numeric: tabular-nums;
}

.thread-toolbar__done:is(:hover, :focus-visible):not(:disabled) {
  outline: var(--border-focus);
  outline-offset: var(--space-2xs);
}

.thread-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-sm);
  height: 56px;
  padding: 0 var(--space-xl);
  flex-shrink: 0;
  -webkit-app-region: drag;
}

.thread-toolbar__position {
  margin-right: var(--space-xs);
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.thread-toolbar__group {
  display: flex;
  align-items: center;
  gap: 4px;
  -webkit-app-region: no-drag;
}

.divider {
  width: 1px;
  height: 24px;
  margin: 0 6px;
  background: var(--border-color);
}

.menu-shortcut {
  min-width: 22px;
  color: inherit;
  font-family: var(--font-mono);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-semibold);
  opacity: .58;
  text-align: center;
}

.submenu-title {
  margin: 3px 10px 5px;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.stacked-label {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.stacked-label strong {
  overflow: hidden;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.stacked-label small {
  overflow: hidden;
  color: inherit;
  font-size: var(--font-size-2xs);
  opacity: 0.65;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.format-icon {
  display: inline-grid;
  place-items: center;
  width: 16px;
  color: inherit;
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  letter-spacing: -0.04em;
  line-height: 1;
}

@media (max-width: 760px) {
  .thread-toolbar { padding-inline: 10px; }
}

.more-wrap {
  display: inline-flex;
}

</style>
