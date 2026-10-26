<script setup lang="ts">
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import IconButton from '@/components/ui/IconButton.vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import PluginSlotHost from '@/components/plugins/PluginSlotHost.vue';
import { useShortcutHints } from '@/composables/useShortcutHints';
import type { AnchorBox } from '@/utils/popoverPlacement';

type Submenu = 'block' | 'move' | 'organize' | 'print' | 'markdown' | 'export';
type MessageScope = 'latest' | 'conversation';
type ExportFormat = 'eml' | 'markdown' | 'text';

const props = withDefaults(
  defineProps<{
    threadId: string;
    subject: string;
    isUnread?: boolean;
    isStarred?: boolean;
    isAside?: boolean;
    /** Done already: the primary action turns into its inverse. */
    isDone?: boolean;
    isMuted?: boolean;
    showBack?: boolean;
    senderAddress?: string;
    moveTargets?: Array<{ id: string; label: string }>;
    canReply?: boolean;
    /**
     * The reader ends in a reply foot. Reply then lives there and in the menu;
     * a second button up here would only compete with it.
     */
    replyInFoot?: boolean;
    /** Number of downloadable attachments across the thread. */
    attachmentCount?: number;
    readerZoom?: number;
    /** Where this thread sits in the list behind it, 1-based. */
    position?: number;
    total?: number;
  }>(),
  {
    isUnread: false,
    isStarred: false,
    isAside: false,
    isDone: false,
    isMuted: false,
    showBack: false,
    senderAddress: '',
    moveTargets: () => [],
    canReply: true,
    replyInFoot: false,
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
  block: [scope: 'address' | 'domain'];
  move: [targetId: string];
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
  organize: [kind: 'tags' | 'assignee' | 'sharing'];
  addToTopic: [];
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
  move: 'thread.toolbar.moveToFolder',
  organize: 'thread.toolbar.organize',
  print: 'thread.toolbar.print',
  markdown: 'thread.toolbar.copyMarkdown',
  export: 'thread.toolbar.exportTo',
};
const submenuLabel = computed(() => (activeSubmenu.value ? t(SUBMENU_LABELS[activeSubmenu.value]) : ''));

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

function run(action: 'toggleRead' | 'togglePin' | 'setAside' | 'forward' | 'toggleMuted' | 'spam' | 'reply' | 'resend' | 'allActions' | 'copyLink' | 'addToTopic') {
  closeMenu();
  if (action === 'toggleRead') emit('toggleRead');
  if (action === 'togglePin') emit('togglePin');
  if (action === 'setAside') emit('setAside');
  if (action === 'forward') emit('forward');
  if (action === 'toggleMuted') emit('toggleMuted');
  if (action === 'spam') emit('spam');
  if (action === 'reply') emit('reply');
  if (action === 'resend') emit('resend');
  if (action === 'allActions') emit('allActions');
  if (action === 'copyLink') emit('copyLink');
  if (action === 'addToTopic') emit('addToTopic');
}

function runBlock(scope: 'address' | 'domain') {
  closeMenu();
  emit('block', scope);
}

function runMove(targetId: string) {
  closeMenu();
  emit('move', targetId);
}

function runOrganize(kind: 'tags' | 'assignee' | 'sharing') {
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
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M10 3L5 8l5 5"/></svg>
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
        :title="$t('commands.moveToInbox')"
        :shortcut="hint('moveInbox')"
        @click="emit('moveToInbox')"
      >
        <template #icon>
          <AppIcon><path d="M8 11V3.4M5.4 8.4 8 11l2.6-2.6" /><path d="M2.8 11.2v1.2a1 1 0 0 0 1 1h8.4a1 1 0 0 0 1-1v-1.2" /></AppIcon>
        </template>
        {{ $t('commands.moveToInbox') }}
      </AppButton>
      <AppButton
        v-else
        variant="primary"
        class="thread-toolbar__done"
        :title="$t('common.done')"
        :shortcut="hint('markDone')"
        @click="emit('archive')"
      >
        <template #icon>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3 8.5l3.2 3.2L13 5"/></svg>
        </template>
        {{ $t('common.done') }}
      </AppButton>
      <IconButton size="lg" :title="$t('thread.toolbar.setReminder')" :shortcut="hint('moveSnoozed')" @click="emit('snooze')">
        <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="8" r="6"/><path d="M8 4v4l2.5 1.5"/></svg>
      </IconButton>
      <IconButton size="lg" tone="danger" :title="$t('common.delete')" :shortcut="hint('trash')" @click="emit('delete')">
        <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 4h12M5 4V2.5A1.5 1.5 0 016.5 1h3A1.5 1.5 0 0111 2.5V4"/><path d="M13 4v9.5a1.5 1.5 0 01-1.5 1.5h-7A1.5 1.5 0 013 13.5V4"/></svg>
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
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M11 3.5 6 8l5 4.5"/></svg>
        </IconButton>
        <IconButton
          size="lg"
          :disabled="position >= total"
          :title="$t('thread.toolbar.next')"
          :shortcut="hint('nextThread')"
          @click="emit('next')"
        >
          <svg width="20" height="20" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 3.5 10 8l-5 4.5"/></svg>
        </IconButton>
      </template>

      <PluginSlotHost slot-name="top_toolbar" :context="{ threadId, subject }" />
      <AppButton v-if="canReply && !replyInFoot" variant="soft" :title="$t('common.reply')" :shortcut="hint('reply')" @click="emit('reply')">
        <template #icon><svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3L1 8l5 5"/><path d="M1 8h9a5 5 0 015 5"/></svg></template>
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
          <svg width="20" height="20" viewBox="0 0 16 16" fill="currentColor"><circle cx="3" cy="8" r="1.5"/><circle cx="8" cy="8" r="1.5"/><circle cx="13" cy="8" r="1.5"/></svg>
        </IconButton>
      </span>
    </div>
  </div>

  <ContextMenu
    :open="menuOpen"
    :anchor="menuAnchor"
    :label="$t('thread.toolbar.moreActions')"
    :width="282"
    @close="closeMenu"
  >
    <div v-if="canReply" class="menu-group" @mouseenter="closeSubmenu">
      <MenuItem :label="$t('common.reply')" :aria-keyshortcuts="hint('reply')?.aria" @select="run('reply')">
        <template #icon><AppIcon :grid="20"><path d="M8 5 3 10l5 5M3 10h8a5 5 0 0 1 5 5" /></AppIcon></template>
        <template v-if="hint('reply')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('reply')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.resend')" @select="run('resend')">
        <template #icon><AppIcon :grid="20"><path d="m3 10 14-7-4.5 14-2.7-5.4L3 10Z" /><path d="m9.8 11.6 3-3" /></AppIcon></template>
      </MenuItem>
      <MenuItem :label="$t('common.forward')" :aria-keyshortcuts="hint('forward')?.aria" @select="run('forward')">
        <template #icon><AppIcon :grid="20"><path d="m12 5 5 5-5 5M17 10H9a5 5 0 0 0-5 5" /></AppIcon></template>
        <template v-if="hint('forward')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('forward')?.label }}</kbd></template>
      </MenuItem>
    </div>

    <div class="menu-group">
      <MenuItem
        :label="$t('thread.toolbar.organize')"
        submenu
        :expanded="activeSubmenu === 'organize'"
        @mouseenter="openSubmenu('organize', $event)"
        @select="openSubmenu('organize', $event)"
      >
        <template #icon><AppIcon :grid="20"><path d="M3 5h14M3 10h14M3 15h14" /><circle cx="7" cy="5" r="1.5" /><circle cx="13" cy="10" r="1.5" /><circle cx="9" cy="15" r="1.5" /></AppIcon></template>
      </MenuItem>
      <MenuItem
        :label="isUnread ? $t('thread.actions.markRead') : $t('thread.actions.markUnread')"
        :aria-keyshortcuts="hint('toggleRead')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('toggleRead')"
      >
        <template #icon><AppIcon :grid="20"><rect x="3" y="5" width="14" height="11" rx="2" /><path d="m4 7 6 4 6-4" /></AppIcon></template>
        <template v-if="hint('toggleRead')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('toggleRead')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="isStarred ? $t('thread.actions.unpin') : $t('thread.actions.pin')"
        :aria-keyshortcuts="hint('pin')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('togglePin')"
      >
        <template #icon><AppIcon :grid="20"><path d="M6.5 3h7l-1 5 2.5 2.5V12H5v-1.5L7.5 8l-1-5Z" /><path d="M10 12v5" /></AppIcon></template>
        <template v-if="hint('pin')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('pin')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="isAside ? $t('thread.actions.moveToInbox') : $t('thread.actions.setAside')"
        :aria-keyshortcuts="hint(isAside ? 'moveInbox' : 'setAside')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('setAside')"
      >
        <template #icon><AppIcon :grid="20"><path d="M3 7h14v9H3z" /><path d="M7 10h6" /><path d="M10 2v5" /><path d="m8 5 2 2 2-2" /></AppIcon></template>
        <template v-if="hint(isAside ? 'moveInbox' : 'setAside')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint(isAside ? 'moveInbox' : 'setAside')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="isMuted ? $t('thread.actions.unmute') : $t('thread.actions.mute')"
        :aria-keyshortcuts="hint('toggleMute')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('toggleMuted')"
      >
        <template #icon><AppIcon :grid="20"><path d="M5 8v4h3l4 3V5L8 8H5Z" /><path v-if="!isMuted" d="M15 7a4 4 0 0 1 0 6" /><path v-else d="m15 8 3 4m0-4-3 4" /></AppIcon></template>
        <template v-if="hint('toggleMute')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('toggleMute')?.label }}</kbd></template>
      </MenuItem>
    </div>

    <div class="menu-group">
      <MenuItem
        :label="$t('common.block')"
        submenu
        :expanded="activeSubmenu === 'block'"
        @mouseenter="openSubmenu('block', $event)"
        @select="openSubmenu('block', $event)"
      >
        <template #icon><AppIcon :grid="20"><path d="M6 4.5A6.5 6.5 0 0 1 16.5 10M5 7H2.5v4H5l3 3V6L5 7Z" /><path d="m4 3 13 14" /></AppIcon></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.move')"
        submenu
        :expanded="activeSubmenu === 'move'"
        @mouseenter="openSubmenu('move', $event)"
        @select="openSubmenu('move', $event)"
      >
        <template #icon><AppIcon :grid="20"><path d="M2.5 6h6l1.7 2H17v8.5H2.5V6Z" /></AppIcon></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.markSpam')"
        tone="danger"
        :aria-keyshortcuts="hint('markSpam')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('spam')"
      >
        <template #icon><AppIcon :grid="20"><path d="M3 5.5h14v11H3v-11ZM6 3.5h8M7 9l6 4m0-4-6 4" /></AppIcon></template>
        <template v-if="hint('markSpam')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('markSpam')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.print')"
        submenu
        :expanded="activeSubmenu === 'print'"
        @mouseenter="openSubmenu('print', $event)"
        @select="openSubmenu('print', $event)"
      >
        <template #icon><AppIcon :grid="20"><path d="M6 7V3h8v4M5 15H3V8h14v7h-2M6 12h8v5H6v-5Z" /><path d="M14.5 10h.01" /></AppIcon></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.link.copy')"
        :aria-keyshortcuts="hint('copyLink')?.aria"
        @mouseenter="closeSubmenu"
        @select="run('copyLink')"
      >
        <template #icon><AppIcon :grid="20"><path d="M8.5 11.5a3.2 3.2 0 0 0 4.6 0l2.6-2.6a3.2 3.2 0 0 0-4.6-4.6l-1.1 1.1" /><path d="M11.5 8.5a3.2 3.2 0 0 0-4.6 0l-2.6 2.6a3.2 3.2 0 0 0 4.6 4.6l1.1-1.1" /></AppIcon></template>
        <template v-if="hint('copyLink')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('copyLink')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.copyMarkdown')"
        submenu
        :expanded="activeSubmenu === 'markdown'"
        @mouseenter="openSubmenu('markdown', $event)"
        @select="openSubmenu('markdown', $event)"
      >
        <template #icon><AppIcon :grid="20"><rect x="5" y="5" width="12" height="12" rx="2" /><path d="M3 14V5a2 2 0 0 1 2-2h9" /></AppIcon></template>
      </MenuItem>
    </div>

    <div class="menu-group" @mouseenter="closeSubmenu">
      <div class="zoom-row" role="group" :aria-label="$t('thread.toolbar.textSize')">
        <AppIcon :grid="20" class="zoom-row__icon"><circle cx="9" cy="9" r="5" /><path d="M13 13l4 4" /></AppIcon>
        <span>{{ $t('thread.toolbar.textSize') }}</span>
        <span class="zoom-controls">
          <IconButton
            class="zoom-btn"
            role="menuitem"
            :title="$t('thread.toolbar.smaller')"
            :aria-label="$t('thread.toolbar.decreaseText')"
            @click.stop="emit('zoom', -10)"
          >−</IconButton>
          <span class="zoom-value">{{ props.readerZoom }}%</span>
          <IconButton
            class="zoom-btn"
            role="menuitem"
            :title="$t('thread.toolbar.larger')"
            :aria-label="$t('thread.toolbar.increaseText')"
            @click.stop="emit('zoom', 10)"
          >+</IconButton>
          <IconButton
            class="zoom-btn"
            role="menuitem"
            :title="$t('common.reset')"
            :aria-label="$t('thread.toolbar.resetTextSize')"
            @click.stop="emit('zoom', 0)"
          >⤺</IconButton>
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
        <template #icon><AppIcon :grid="20"><path d="M10 3v10m0 0-4-4m4 4 4-4" /><path d="M4 15v2h12v-2" /></AppIcon></template>
      </MenuItem>
      <MenuItem
        :label="$t('thread.toolbar.exportTo')"
        submenu
        :expanded="activeSubmenu === 'export'"
        @mouseenter="openSubmenu('export', $event)"
        @select="openSubmenu('export', $event)"
      >
        <template #icon><AppIcon :grid="20"><path d="M10 13V3m0 0L6 7m4-4 4 4" /><path d="M4 10v7h12v-7" /></AppIcon></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.allActions')" :aria-keyshortcuts="hint('openCommandPalette')?.aria" @mouseenter="closeSubmenu" @select="run('allActions')">
        <template #icon><AppIcon :grid="20"><rect x="3" y="4" width="14" height="12" rx="2" /><path d="M6 8h8M6 12h5" /></AppIcon></template>
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
      <MenuItem :label="$t('thread.actions.addToTopic')" :aria-keyshortcuts="hint('addToTopic')?.aria" @select="run('addToTopic')">
        <template v-if="hint('addToTopic')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('addToTopic')?.label }}</kbd></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.editTags')" @select="runOrganize('tags')" />
      <MenuItem :label="$t('thread.toolbar.changeAssignee')" @select="runOrganize('assignee')" />
      <MenuItem :label="$t('thread.toolbar.changeSharing')" @select="runOrganize('sharing')" />
    </template>

    <template v-else-if="activeSubmenu === 'block'">
      <p class="submenu-title">{{ $t('thread.actions.blockSender') }}</p>
      <MenuItem :label="$t('thread.toolbar.blockAddress')" @select="runBlock('address')">
        <template #icon><AppIcon :grid="20"><circle cx="10" cy="7" r="3" /><path d="M4 17a6 6 0 0 1 12 0" /></AppIcon></template>
        <span class="stacked-label"><strong>{{ $t('thread.toolbar.blockAddress') }}</strong><small>{{ senderAddress || $t('thread.toolbar.currentSender') }}</small></span>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.blockDomain')" @select="runBlock('domain')">
        <template #icon><AppIcon :grid="20"><circle cx="10" cy="10" r="7" /><path d="M3 10h14M10 3a11 11 0 0 1 0 14M10 3a11 11 0 0 0 0 14" /></AppIcon></template>
      </MenuItem>
    </template>

    <template v-else-if="activeSubmenu === 'move'">
      <p class="submenu-title">{{ $t('thread.toolbar.moveToFolder') }}</p>
      <MenuItem
        v-for="target in props.moveTargets"
        :key="target.id"
        :label="target.label"
        @select="runMove(target.id)"
      >
        <template #icon><AppIcon :grid="20"><path d="M2.5 6h6l1.7 2H17v8.5H2.5V6Z" /></AppIcon></template>
      </MenuItem>
      <p v-if="props.moveTargets.length === 0" class="submenu-empty">{{ $t('thread.toolbar.noOtherFolders') }}</p>
    </template>

    <template v-else-if="activeSubmenu === 'print'">
      <MenuItem :label="$t('thread.toolbar.latestMessageOnly')" @select="runPrint('latest')">
        <template #icon><AppIcon :grid="20"><path d="M6 7V3h8v4M5 15H3V8h14v7h-2M6 12h8v5H6v-5Z" /></AppIcon></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.wholeConversation')" :aria-keyshortcuts="hint('print')?.aria" @select="runPrint('conversation')">
        <template #icon><AppIcon :grid="20"><rect x="3" y="3" width="11" height="11" rx="2" /><path d="M7 17h8a2 2 0 0 0 2-2V7" /></AppIcon></template>
        <template v-if="hint('print')" #trailing><kbd class="menu-shortcut" aria-hidden="true">{{ hint('print')?.label }}</kbd></template>
      </MenuItem>
    </template>

    <template v-else-if="activeSubmenu === 'markdown'">
      <MenuItem :label="$t('thread.toolbar.copyLatest')" @select="runCopyMarkdown('latest')">
        <template #icon><AppIcon :grid="20"><rect x="5" y="5" width="12" height="12" rx="2" /><path d="M3 14V5a2 2 0 0 1 2-2h9" /></AppIcon></template>
      </MenuItem>
      <MenuItem :label="$t('thread.toolbar.copyConversation')" @select="runCopyMarkdown('conversation')">
        <template #icon><AppIcon :grid="20"><rect x="3" y="3" width="11" height="11" rx="2" /><path d="M7 17h8a2 2 0 0 0 2-2V7" /></AppIcon></template>
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

.submenu-empty {
  margin: 9px 10px;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
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
