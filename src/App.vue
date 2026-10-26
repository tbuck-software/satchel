<script setup lang="ts">
import { computed, defineAsyncComponent, watch } from 'vue';
import { RouterView, useRoute } from 'vue-router';
import Sidebar from '@/components/layout/Sidebar.vue';
import ComposeFab from '@/components/layout/ComposeFab.vue';
import ToastStack from '@/components/ui/ToastStack.vue';
import PaneResizeHandle from '@/components/ui/PaneResizeHandle.vue';
import UndoFab from '@/components/ui/UndoFab.vue';
import AccountReauthBanner from '@/components/settings/AccountReauthBanner.vue';
import ConnectionBanner from '@/components/layout/ConnectionBanner.vue';
import SyncStatusLine from '@/components/layout/SyncStatusLine.vue';
import { useComposerStore } from '@/stores/composer';
import { useConfirm } from '@/composables/useConfirm';
import { usePrompt } from '@/composables/usePrompt';
import { useTopicsStore } from '@/stores/topics';
import { useSettingsStore } from '@/stores/settings';
import { SIDEBAR_WIDTH_LIMITS, useUiStore } from '@/stores/ui';
import { useAppBootstrap, useComposeWindowBootstrap } from '@/bootstrap';
import { loadOmniSearch } from '@/components/navigation/loadOmniSearch';

const route = useRoute();
const topicsStore = useTopicsStore();
const settingsStore = useSettingsStore();
const ui = useUiStore();
const composer = useComposerStore();
const { state: confirmState } = useConfirm();
const { state: promptState } = usePrompt();
const standaloneCompose = computed(() =>
  route.name === 'compose' && route.query.standalone === '1',
);

// These surfaces are absent during the common mailbox startup. Keeping both
// the async boundary and the outer v-if is intentional: an async component is
// fetched as soon as Vue mounts it, even when its own template renders nothing.
const loadComposerHost = () => import('@/components/mail/ComposerHost.vue');
const ComposerHost = defineAsyncComponent(loadComposerHost);
const OmniSearch = defineAsyncComponent(loadOmniSearch);
const ShortcutHelper = defineAsyncComponent(() => import('@/components/navigation/ShortcutHelper.vue'));
const TopicPickerOverlay = defineAsyncComponent(() => import('@/components/topics/TopicPickerOverlay.vue'));
const OnboardingFlow = defineAsyncComponent(() => import('@/components/onboarding/OnboardingFlow.vue'));
const SettingsModal = defineAsyncComponent(() => import('@/components/settings/SettingsModal.vue'));
// The shared yes/no and one-line dialogs: their state lives in `useConfirm` /
// `usePrompt`, so the frame (ModalShell, focus trap) loads with the first question.
const ConfirmDialog = defineAsyncComponent(() => import('@/components/ui/ConfirmDialog.vue'));
const PromptDialog = defineAsyncComponent(() => import('@/components/ui/PromptDialog.vue'));

watch(() => route.fullPath, () => {
  if (ui.mobileSidebarOpen) {
    ui.closeMobileSidebar();
  }
});

// Decided once, at setup: a window either is a composer popout or it is the
// application. Reacting to the route instead would boot the full application —
// stores, listeners and a second auto-sync loop — inside the popout the moment
// a send navigates it away from `/compose`.
if (standaloneCompose.value) {
  useComposeWindowBootstrap();
} else {
  useAppBootstrap();
}
</script>

<template>
  <div
    :class="['shell', { 'shell--mobile': ui.isMobile, 'shell--composer-standalone': standaloneCompose }]"
    :style="!ui.isMobile ? { '--sidebar-w': `${ui.sidebarWidth}px` } : undefined"
  >
    <!-- Desktop: sidebar + resize handle -->
    <template v-if="!ui.isMobile && !standaloneCompose">
      <Sidebar />
      <PaneResizeHandle
        :label="$t('ui.resizeSidebar')"
        :value="ui.sidebarWidth"
        :min="SIDEBAR_WIDTH_LIMITS.min"
        :max="SIDEBAR_WIDTH_LIMITS.max"
        @resize="ui.setSidebarWidth"
      />
    </template>

    <div class="main-shell">
      <div
        v-if="!standaloneCompose"
        class="window-drag-strip desktop-window-chrome"
        data-tauri-drag-region
      />

      <AccountReauthBanner v-if="!standaloneCompose" />
      <ConnectionBanner v-if="!standaloneCompose" />

      <!-- On the phone the sidebar's sync button lives in a closed drawer.
           Keep connection status visible here, with resting state suppressed. -->
      <SyncStatusLine
        v-if="ui.isMobile && !standaloneCompose"
        class="mobile-sync-status"
        hide-idle
      />

      <main>
        <RouterView />
      </main>
    </div>

    <ComposerHost v-if="composer.isOpen" />

    <!-- Mobile: bottom navigation (hidden on thread detail — ThreadView has its own action bar) -->
    <!-- The bottom bar held Inbox, Schreiben and Menu. Inbox is where you
         already are, Schreiben is an action rather than a place, and the drawer
         belongs under the thumb at the top-left of the header — so the bar
         itself had nothing left to carry. -->
    <ComposeFab
      v-if="ui.isMobile && !standaloneCompose && route.name !== 'thread' && route.name !== 'settings'"
    />

    <!-- Mobile: sidebar overlay -->
    <Transition name="sidebar-slide">
      <div v-if="ui.mobileSidebarOpen" class="mobile-sidebar-overlay">
        <div class="mobile-sidebar-panel">
          <Sidebar />
        </div>
        <div class="mobile-sidebar-backdrop" @click="ui.closeMobileSidebar()" />
      </div>
    </Transition>

    <OmniSearch v-if="ui.omniOpen" />
    <ShortcutHelper v-if="ui.shortcutsOpen" />
    <TopicPickerOverlay v-if="topicsStore.pickerTarget" />
    <SettingsModal v-if="ui.settingsOpen" />
    <OnboardingFlow v-if="!settingsStore.onboardingComplete || ui.accountSetupOpen" />
    <Transition name="key-sequence">
      <div v-if="ui.keySequenceHint" class="key-sequence-hint" role="status">
        <kbd>g</kbd>
        <span>{{ ui.keySequenceHint }}</span>
      </div>
    </Transition>
    <ToastStack />
    <ConfirmDialog v-if="confirmState.pending" />
    <PromptDialog v-if="promptState.pending" />
    <UndoFab :variant="ui.isSinglePane ? 'floating' : 'docked'" />
  </div>
</template>

<style scoped>
.shell {
  position: relative;
  height: 100%;
  display: grid;
  grid-template-columns: var(--sidebar-w) 1px minmax(0, 1fr);
  overflow: hidden;
  background: var(--bg-base);
  padding:
    var(--satchel-safe-area-top)
    var(--satchel-safe-area-right)
    var(--satchel-safe-area-bottom)
    var(--satchel-safe-area-left);
}

.key-sequence-hint {
  position: fixed;
  left: 50%;
  bottom: calc(28px + var(--satchel-safe-area-bottom));
  z-index: 1100;
  display: flex;
  align-items: center;
  gap: 10px;
  max-width: min(620px, calc(100vw - 32px));
  padding: 8px 12px 8px 9px;
  border: 1px solid color-mix(in srgb, var(--accent-primary) 34%, var(--border-color));
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--bg-overlay) 94%, transparent);
  color: var(--text-secondary);
  box-shadow: var(--shadow-dropdown);
  font-size: var(--font-size-xs);
  transform: translateX(-50%);
  backdrop-filter: blur(14px);
}

.key-sequence-hint kbd {
  display: grid;
  width: 24px;
  height: 24px;
  place-items: center;
  border-radius: var(--radius-xs);
  background: var(--accent-primary);
  color: var(--text-on-accent);
  font-family: var(--font-mono);
  font-weight: var(--weight-bold);
}

.key-sequence-enter-active,
.key-sequence-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out), transform var(--duration-fast) var(--ease-out);
}

.key-sequence-enter-from,
.key-sequence-leave-to {
  opacity: 0;
  transform: translate(-50%, 8px);
}

.shell--mobile {
  grid-template-columns: 1fr;
  grid-template-rows: 1fr;
}

.shell--composer-standalone {
  grid-template-columns: 1fr;
  grid-template-rows: 1fr;
}

main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

/*
 * Flex rather than fixed grid rows: the drag strip and the reauth banner both
 * come and go, and with `grid-template-rows: auto 1fr` a missing one pushed
 * main into the wrong row and it stopped filling the shell.
 */
.main-shell {
  position: relative;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-surface);
}

.shell--mobile .main-shell {
  grid-template-rows: 1fr;
}

.mobile-sync-status {
  flex-shrink: 0;
  border-bottom: 1px solid var(--border-color-muted);
}

.window-drag-strip {
  height: var(--satchel-window-chrome-height);
  flex-shrink: 0;
  background: var(--bg-base);
  -webkit-app-region: drag;
}

/* Mobile sidebar overlay */
.mobile-sidebar-overlay {
  position: fixed;
  inset:
    var(--satchel-safe-area-top)
    var(--satchel-safe-area-right)
    var(--satchel-safe-area-bottom)
    var(--satchel-safe-area-left);
  z-index: 300;
  display: flex;
}

.mobile-sidebar-panel {
  width: 300px;
  max-width: 80vw;
  height: 100%;
  background: var(--bg-sidebar);
  box-shadow: 4px 0 24px rgba(0, 0, 0, 0.4);
  z-index: 1;
  overflow: hidden;
}

.mobile-sidebar-backdrop {
  flex: 1;
  background: rgba(0, 0, 0, 0.5);
}

/* Sidebar slide transition */
.sidebar-slide-enter-active {
  transition: opacity var(--duration-normal) var(--ease-out);
}

.sidebar-slide-enter-active .mobile-sidebar-panel {
  transition: transform var(--duration-slow) var(--ease-out);
}

.sidebar-slide-leave-active {
  transition: opacity var(--duration-normal) var(--ease-out);
}

.sidebar-slide-leave-active .mobile-sidebar-panel {
  transition: transform var(--duration-normal) var(--ease-out);
}

.sidebar-slide-enter-from {
  opacity: 0;
}

.sidebar-slide-enter-from .mobile-sidebar-panel {
  transform: translateX(-100%);
}

.sidebar-slide-leave-to {
  opacity: 0;
}

.sidebar-slide-leave-to .mobile-sidebar-panel {
  transform: translateX(-100%);
}
</style>
