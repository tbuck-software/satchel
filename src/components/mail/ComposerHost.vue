<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useI18n } from 'vue-i18n';
import ComposeView from '@/views/ComposeView.vue';
import { useComposerStore, type ComposerRequest } from '@/stores/composer';
import { useDraftsStore } from '@/stores/drafts';
import { useUiStore } from '@/stores/ui';
import { useToast } from '@/composables/useToast';
import { useConfirm } from '@/composables/useConfirm';
import { useFocusTrap } from '@/composables/useFocusTrap';
import { COMPOSER_DEFAULT_SIZE, COMPOSER_MINIMUM_SIZE, COMPOSER_DOCK_GAP, fitComposerRect, dockComposerRect, resizeComposerRect, type ResizeDirection } from '@/utils/composerGeometry';
import { useComposeWindow } from '@/composables/useComposeWindow';
import { refreshToastClearance } from '@/composables/useToastClearance';

interface ComposeViewHandle {
  persistDraft: (options?: { silent?: boolean }) => Promise<string | null>;
  shouldConfirmClose: () => boolean;
  closeWithoutSaving: () => Promise<void>;
  currentDraftId: () => string | null;
}

const GAP = COMPOSER_DOCK_GAP;
const RESIZE_DIRECTIONS: ResizeDirection[] = ['n', 'ne', 'e', 'se', 's', 'sw', 'w', 'nw'];
/** Long enough to notice the toast and change your mind about the draft. */
const UNDO_TIMEOUT_MS = 9000;

const { t } = useI18n();
const composer = useComposerStore();
const draftsStore = useDraftsStore();
const toast = useToast();
const confirm = useConfirm();
const ui = useUiStore();
const router = useRouter();
const composeWindow = useComposeWindow(router);
const layerRef = ref<HTMLElement | null>(null);
const windowRef = ref<HTMLElement | null>(null);
const formRef = ref<ComposeViewHandle | null>(null);
const width = ref<number>(COMPOSER_DEFAULT_SIZE.width);
const height = ref<number>(COMPOSER_DEFAULT_SIZE.height);
const x = ref(GAP);
const y = ref(GAP);
const maximized = ref(false);
const dragging = ref(false);
const resizing = ref(false);
const resizeDirection = ref<ResizeDirection>('se');
// Geometry belongs to this open composer. Closing resets the next message to
// the bottom-right dock; dragging or resizing only affects the current message.
const manuallyPlaced = ref(false);
const popoutError = ref('');
const closeBusy = ref(false);

let startPointerX = 0;
let startPointerY = 0;
let startX = 0;
let startY = 0;
let startWidth = 0;
let startHeight = 0;

const request = computed(() => composer.request);

// The composer is modal in practice: it covers the app, Escape leaves it, and
// `data-composer-surface` already keeps the global single-key shortcuts out.
// Tabbing out of it landed in the thread list underneath, where every keystroke
// then meant something else. ComposeView picks the field it starts in.
const trap = useFocusTrap(windowRef, {
  isOpen: () => composer.isOpen,
  moveFocusIn: () => false,
});
const composerHeading = computed(() => {
  const mode = request.value?.mode;
  if (mode === 'forward') return t('common.forward');
  if (mode === 'resend') return t('compose.title.resend');
  if (mode?.startsWith('reply')) return t('common.reply');
  return t('compose.title.new');
});
const windowStyle = computed(() => {
  if (maximized.value || ui.isMobile) {
    return {
      inset: ui.isMobile ? '0' : `${GAP}px`,
    };
  }
  return {
    width: `${width.value}px`,
    height: `${height.value}px`,
    transform: `translate3d(${x.value}px, ${y.value}px, 0)`,
  };
});

function layerBounds() {
  const rect = layerRef.value?.getBoundingClientRect();
  return {
    width: rect?.width || window.innerWidth,
    height: rect?.height || window.innerHeight,
  };
}

function fitIntoLayer() {
  const next = fitComposerRect(
    { x: x.value, y: y.value, width: width.value, height: height.value },
    layerBounds(),
    COMPOSER_MINIMUM_SIZE,
  );
  ({ x: x.value, y: y.value, width: width.value, height: height.value } = next);
}

function placeInitially() {
  const next = dockComposerRect(layerBounds(), COMPOSER_DEFAULT_SIZE, COMPOSER_MINIMUM_SIZE, GAP);
  ({ x: x.value, y: y.value, width: width.value, height: height.value } = next);
}

function beginDrag(event: PointerEvent) {
  if (maximized.value || ui.isMobile || (event.target as HTMLElement).closest('button')) return;
  event.preventDefault();
  dragging.value = true;
  startPointerX = event.clientX;
  startPointerY = event.clientY;
  startX = x.value;
  startY = y.value;
  window.addEventListener('pointermove', moveDrag);
  window.addEventListener('pointerup', endDrag, { once: true });
}

function moveDrag(event: PointerEvent) {
  const bounds = layerBounds();
  x.value = Math.max(0, Math.min(startX + event.clientX - startPointerX, bounds.width - width.value));
  y.value = Math.max(0, Math.min(startY + event.clientY - startPointerY, bounds.height - height.value));
}

function endDrag() {
  dragging.value = false;
  window.removeEventListener('pointermove', moveDrag);
  manuallyPlaced.value = true;
}

function beginResize(event: PointerEvent, direction: ResizeDirection) {
  if (maximized.value || ui.isMobile) return;
  event.preventDefault();
  resizing.value = true;
  resizeDirection.value = direction;
  startPointerX = event.clientX;
  startPointerY = event.clientY;
  startX = x.value;
  startY = y.value;
  startWidth = width.value;
  startHeight = height.value;
  window.addEventListener('pointermove', moveResize);
  window.addEventListener('pointerup', endResize, { once: true });
}

function moveResize(event: PointerEvent) {
  const next = resizeComposerRect(
    { x: startX, y: startY, width: startWidth, height: startHeight },
    resizeDirection.value,
    event.clientX - startPointerX,
    event.clientY - startPointerY,
    layerBounds(),
    COMPOSER_MINIMUM_SIZE,
  );
  x.value = next.x;
  y.value = next.y;
  width.value = next.width;
  height.value = next.height;
}

function endResize() {
  resizing.value = false;
  window.removeEventListener('pointermove', moveResize);
  fitIntoLayer();
  manuallyPlaced.value = true;
}

/**
 * Closing keeps the work instead of asking about it. The composer already
 * autosaves every ten seconds, so by the time anyone clicks the X the draft is
 * usually on disk already — the dialog was asking about something that had
 * happened. The one case that still stops the close is a save that fails:
 * dropping the window then would destroy text with nowhere else to be.
 */
// Back should close the composer — with the same guard the close button uses,
// so a draft with unsaved content still gets its confirmation instead of
// vanishing on a swipe.
onBeforeUnmount(
  ui.registerOverlay({
    isOpen: () => composer.isOpen,
    dismiss: () => void requestCloseComposer(),
  }),
);

/** Resolves `true` once the composer is gone, `false` when it had to stay. */
async function requestCloseComposer(): Promise<boolean> {
  if (!formRef.value?.shouldConfirmClose()) {
    await formRef.value?.closeWithoutSaving();
    composer.close();
    return true;
  }

  closeBusy.value = true;
  popoutError.value = '';
  try {
    const savedDraftId = await formRef.value?.persistDraft({ silent: true });
    if (!savedDraftId) {
      // The window stays open on purpose — dropping it would take the text
      // with it. But the thin line under the title bar was the only sign, so
      // the X read as broken rather than as "your draft is not on disk".
      popoutError.value = t('compose.draft.saveFailedStaysOpen');
      toast.error(t('compose.draft.saveFailedStaysOpen'), {
        action: {
          label: t('common.retry'),
          handler: () => void requestCloseComposer(),
        },
      });
      return false;
    }
    composer.close();
    toast.info(t('compose.draft.savedAsDraft'), {
      timeout: UNDO_TIMEOUT_MS,
      action: { label: t('common.discard'), handler: () => void discardSavedDraft(savedDraftId) },
    });
    return true;
  } finally {
    closeBusy.value = false;
  }
}

/**
 * A second request (reply, mailto, tray, notification) while a composer is
 * open. Untouched composers make way; one with work in it asks first and is
 * then saved through the regular close path, which keeps it open if that fails.
 */
async function confirmReplace(next: ComposerRequest): Promise<boolean> {
  const form = formRef.value;
  if (!form) return true;
  // The same draft is already in front of the user.
  if (next.draftId && next.draftId === form.currentDraftId()) return false;
  if (!form.shouldConfirmClose()) {
    await form.closeWithoutSaving();
    return true;
  }
  const replace = await confirm.ask({
    title: t('compose.replace.title'),
    body: t('compose.replace.body'),
    confirmLabel: t('compose.replace.confirm'),
  });
  if (!replace) return false;
  return requestCloseComposer();
}

onBeforeUnmount(composer.setReplaceGuard(confirmReplace));

async function discardSavedDraft(draftId: string) {
  try {
    await draftsStore.remove(draftId);
    toast.info(t('compose.draft.discarded'));
  } catch (error) {
    toast.error(
      t('compose.draft.discardFailed', {
        error: error instanceof Error ? error.message : String(error),
      }),
    );
  }
}

function toggleMaximize() {
  maximized.value = !maximized.value;
  if (!maximized.value) {
    nextTick(() => {
      if (manuallyPlaced.value) fitIntoLayer();
      else placeInitially();
    });
  }
}

async function openInOwnWindow() {
  popoutError.value = '';
  try {
    const draftId = await formRef.value?.persistDraft({ silent: true });
    if (!draftId && formRef.value?.shouldConfirmClose()) {
      popoutError.value = t('compose.draft.saveFailedStaysOpen');
      return;
    }
    await composeWindow.open(draftId ?? null);
    // The popout owns the draft now; a save from here would race its first edit.
    await formRef.value?.closeWithoutSaving();
    composer.close();
  } catch (error) {
    popoutError.value = t('compose.window.popOutFailed', {
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

function finishComposer() {
  composer.close();
}

function onResize() {
  if (!maximized.value && composer.isOpen) {
    if (manuallyPlaced.value) fitIntoLayer();
    else placeInitially();
  }
}

watch(
  () => composer.instance,
  async () => {
    if (!composer.isOpen) return;
    popoutError.value = '';
    closeBusy.value = false;
    maximized.value = false;
    await nextTick();
    manuallyPlaced.value = false;
    placeInitially();
  },
  // App mounts this host only after opening, so the initial request already
  // exists when setup runs. A non-immediate watcher never places that window.
  { immediate: true },
);

// A drag moves the send row by transform, which no layout event reports;
// a toast that is already up has to follow it.
watch([x, y, width, height, maximized], refreshToastClearance, { flush: 'post' });

if (typeof window !== 'undefined') window.addEventListener('resize', onResize);

onBeforeUnmount(() => {
  window.removeEventListener('resize', onResize);
  window.removeEventListener('pointermove', moveDrag);
  window.removeEventListener('pointermove', moveResize);
});
</script>

<template>
  <!-- `data-composer-surface` is the contract useKeyboard reads: clicks in here
       keep their focus, and global single-key shortcuts stay out. -->
  <div ref="layerRef" class="composer-layer" data-composer-surface aria-live="polite">
    <Transition name="composer-window">
      <section
        v-if="request"
        ref="windowRef"
        :key="composer.instance"
        :class="[
          'composer-window',
          {
            'composer-window--maximized': maximized || ui.isMobile,
            'composer-window--moving': dragging || resizing,
          },
        ]"
        :style="windowStyle"
        role="dialog"
        aria-modal="true"
        :aria-label="$t('compose.window.ariaLabel')"
        @keydown.tab="trap.onTab"
      >
        <header class="composer-chrome" @pointerdown="beginDrag" @dblclick="toggleMaximize">
          <!-- On a phone the composer fills the screen, so leaving it is going
               back, not closing a window. The X stays for the desktop, where it
               really is one. -->
          <IconButton
            v-if="ui.isMobile"
            class="composer-back"
            :aria-label="$t('common.back')"
            @click.stop="requestCloseComposer"
           :title="$t('common.back')">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M15 18l-6-6 6-6" />
            </svg>
          </IconButton>
          <div class="composer-heading">
            <strong>{{ composerHeading }}</strong>
          </div>
          <div v-if="!ui.isMobile" class="composer-window-actions">
            <IconButton
              :title="maximized ? $t('compose.window.restore') : $t('compose.window.maximize')"
              :aria-label="maximized ? $t('compose.window.restore') : $t('compose.window.maximize')"
              @click.stop="toggleMaximize"
            >
              <AppIcon v-if="!maximized"><path d="M3 6V3h3M10 3h3v3M13 10v3h-3M6 13H3v-3"/></AppIcon>
              <AppIcon v-else><rect x="2.5" y="5" width="8.5" height="8.5" rx="1.5"/><path d="M5 5V3.5A1.5 1.5 0 016.5 2h6A1.5 1.5 0 0114 3.5v6A1.5 1.5 0 0112.5 11H11"/></AppIcon>
            </IconButton>
            <IconButton :title="$t('compose.window.popOut')" :aria-label="$t('compose.window.popOut')" @click.stop="openInOwnWindow">
              <AppIcon><rect x="2" y="4.5" width="9.5" height="9.5" rx="2"/><path d="M6 2h8v8M13.5 2.5L8 8"/></AppIcon>
            </IconButton>
            <IconButton :title="$t('compose.window.close')" :aria-label="$t('compose.window.close')" @click.stop="requestCloseComposer">
              <AppIcon><path d="M3 3l10 10M13 3L3 13"/></AppIcon>
            </IconButton>
          </div>
        </header>

        <p v-if="popoutError" class="composer-error" role="alert">{{ popoutError }}</p>
        <ComposeView
          ref="formRef"
          embedded
          :mode="request.mode"
          :thread-id="request.threadId ?? null"
          :initial-draft-id="request.draftId ?? null"
          :initial-to="request.to ?? []"
          :initial-cc="request.cc ?? []"
          :initial-bcc="request.bcc ?? []"
          :initial-body-html="request.bodyHtml ?? null"
          :initial-subject="request.subject ?? ''"
          :initial-body="request.body ?? ''"
          @update:title="composer.setTitle"
          @sent="finishComposer"
          @close="finishComposer"
        />

        <template v-if="!maximized && !ui.isMobile">
          <div
            v-for="direction in RESIZE_DIRECTIONS"
            :key="direction"
            aria-hidden="true"
            :class="['composer-resize-zone', `composer-resize-zone--${direction}`]"
            @pointerdown.stop="beginResize($event, direction)"
          />
        </template>
      </section>
    </Transition>
  </div>
</template>

<style scoped>
.composer-layer {
  position: absolute;
  inset: 0;
  z-index: 240;
  pointer-events: none;
  overflow: hidden;
}

.composer-window {
  position: absolute;
  top: 0;
  left: 0;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  pointer-events: auto;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, white 8%);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  box-shadow: 0 28px 80px rgba(0, 0, 0, 0.48), 0 2px 10px rgba(0, 0, 0, 0.32), 0 1px 0 rgba(255, 255, 255, 0.035) inset;
  backdrop-filter: blur(28px) saturate(125%);
  -webkit-backdrop-filter: blur(28px) saturate(125%);
  transition: transform var(--duration-spatial) var(--ease-out), width var(--duration-slow) var(--ease-out), height var(--duration-slow) var(--ease-out), inset var(--duration-slow) var(--ease-out), border-radius var(--duration-slow) var(--ease-out);
}

/* Full bleed on a phone, but not underneath the status bar or the gesture
   pill: inset: 0 puts the chrome under the clock and the send row under the
   navigation bar. */
.composer-window--maximized {
  padding:
    var(--satchel-safe-area-top)
    var(--satchel-safe-area-right)
    var(--satchel-safe-area-bottom)
    var(--satchel-safe-area-left);
}

.composer-back {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  margin-right: 2px;
  border: 0;
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  background: transparent;
  cursor: pointer;
}

.composer-window > :deep(.compose) {
  flex: 1;
  min-height: 0;
}

.composer-window--moving {
  transition: none;
  user-select: none;
}

.composer-window--maximized {
  border-radius: var(--radius-md);
}

.composer-chrome {
  min-width: 0;
  height: 58px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 0 12px 0 22px;
  border-bottom: 1px solid var(--border-color-muted);
  cursor: grab;
  touch-action: none;
}

.composer-chrome:active { cursor: grabbing; }

.composer-heading { min-width: 0; }

.composer-heading strong {
  min-width: 0;
  overflow: hidden;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-strong);
  letter-spacing: 0.01em;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.composer-window-actions {
  flex: none;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 3px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-base) 44%, transparent);
}

.composer-window-actions button {
  width: 34px;
  height: 32px;
  display: grid;
  place-items: center;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--text-tertiary);
  cursor: pointer;
}

.composer-window-actions button:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.composer-error {
  margin: 0;
  padding: 8px 18px;
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 9%, var(--bg-surface));
  font-size: var(--font-size-xs);
}

.composer-resize-zone {
  position: absolute;
  z-index: 5;
  touch-action: none;
}

.composer-resize-zone--n,
.composer-resize-zone--s {
  left: 12px;
  right: 12px;
  height: 7px;
  cursor: ns-resize;
}
.composer-resize-zone--n { top: 0; }
.composer-resize-zone--s { bottom: 0; }

.composer-resize-zone--e,
.composer-resize-zone--w {
  top: 12px;
  bottom: 12px;
  width: 7px;
  cursor: ew-resize;
}
.composer-resize-zone--e { right: 0; }
.composer-resize-zone--w { left: 0; }

.composer-resize-zone--ne,
.composer-resize-zone--se,
.composer-resize-zone--sw,
.composer-resize-zone--nw {
  width: 14px;
  height: 14px;
}
.composer-resize-zone--ne { top: 0; right: 0; cursor: nesw-resize; }
.composer-resize-zone--se { right: 0; bottom: 0; cursor: nwse-resize; }
.composer-resize-zone--sw { bottom: 0; left: 0; cursor: nesw-resize; }
.composer-resize-zone--nw { top: 0; left: 0; cursor: nwse-resize; }

.composer-window-enter-active,
.composer-window-leave-active {
  transition: opacity var(--duration-normal) var(--ease-out), transform var(--duration-slow) var(--ease-out);
}

.composer-window-enter-from,
.composer-window-leave-to {
  opacity: 0;
  transform: translate3d(v-bind('x + "px"'), calc(v-bind('y + "px"') + 24px), 0) scale(0.97) !important;
}

@media (max-width: 767px) {
  .composer-window {
    border: 0;
    border-radius: 0;
  }

  .composer-chrome {
    height: 52px;
    padding-left: 16px;
  }

  .composer-window-actions button:first-child { display: none; }
}

@media (prefers-reduced-motion: reduce) {
  .composer-window,
  .composer-window-enter-active,
  .composer-window-leave-active { transition: none; }
}
</style>
