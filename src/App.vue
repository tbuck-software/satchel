<script setup lang="ts">
import { computed, defineAsyncComponent, provide, ref, watch } from 'vue';
import { RouterView, useRoute } from 'vue-router';
import Sidebar from '@/components/layout/Sidebar.vue';
import PaneResizeHandle from '@/components/ui/PaneResizeHandle.vue';
import AccountReauthBanner from '@/components/settings/AccountReauthBanner.vue';
import { useComposerStore } from '@/stores/composer';
import { useToast } from '@/composables/useToast';
import { useConfirm } from '@/composables/useConfirm';
import { usePrompt } from '@/composables/usePrompt';
import { useTopicsStore } from '@/stores/topics';
import { useSettingsStore } from '@/stores/settings';
import { SIDEBAR_WIDTH_LIMITS, useUiStore } from '@/stores/ui';
import { useAppBootstrap, useComposeWindowBootstrap } from '@/bootstrap';
import { loadOmniSearch } from '@/components/navigation/loadOmniSearch';
import { keyboardVisible } from '@/mobile/keyboardInset';
import { IMAGE_GUARD_PHONE_ACTIONS } from '@/components/mail/imageGuardActions';

const route = useRoute();
/**
 * Screens without the compose FAB: a thread has its own action bar, and
 * Settings, Calendar and Contacts have a primary action of their own that a
 * mail pencil would only compete with.
 */
const COMPOSE_FAB_HIDDEN_ON = new Set(['thread', 'settings', 'calendar', 'contacts']);
/**
 * Screens without the phone's tab bar: pushed details with a Back of their own,
 * and those with their own bottom bar (a thread's actions, the composer's send).
 */
const TAB_BAR_HIDDEN_ON = new Set(['thread', 'compose', 'search', 'topic', 'new-senders']);
const topicsStore = useTopicsStore();
const settingsStore = useSettingsStore();
const ui = useUiStore();
const composer = useComposerStore();
const toast = useToast();
const toastMounted = ref(false);
watch(() => toast.state.toasts.length, (count) => { if (count > 0) toastMounted.value = true; }, { immediate: true });
const { state: confirmState } = useConfirm();
const { state: promptState } = usePrompt();
const standaloneCompose = computed(() =>
  route.name === 'compose' && route.query.standalone === '1',
);
// Steps aside while the keyboard is up: it holds no focus, so hiding it cannot
// blur the field being typed in, and the field gets the height back.
const tabBarVisible = computed(() =>
  ui.isMobile
  && !standaloneCompose.value
  && !composer.isOpen
  && !keyboardVisible.value
  && !TAB_BAR_HIDDEN_ON.has(String(route.name)),
);
const mailboxSwitcherMounted = ref(false);
watch(() => ui.mailboxSwitcherOpen, (open) => { if (open) mailboxSwitcherMounted.value = true; });

// These surfaces are absent during the common mailbox startup. Keeping both
// the async boundary and the outer v-if is intentional: an async component is
// fetched as soon as Vue mounts it, even when its own template renders nothing.
const loadComposerHost = () => import('@/components/mail/ComposerHost.vue');
const ComposerHost = defineAsyncComponent(loadComposerHost);
const ToastStack = defineAsyncComponent(() => import('@/components/ui/ToastStack.vue'));
const UndoFab = defineAsyncComponent(() => import('@/components/ui/UndoFab.vue'));
// The phone's buttons on a blocked-images notice; the viewer has no provider.
provide(
  IMAGE_GUARD_PHONE_ACTIONS,
  defineAsyncComponent(() => import('@/components/mail/ImageGuardPhoneActions.vue')),
);
const SyncStatusLine = defineAsyncComponent(() => import('@/components/layout/SyncStatusLine.vue'));
const OmniSearch = defineAsyncComponent(loadOmniSearch);
// Phone-only, and shown only while the backend reports trouble: neither is
// needed to draw the first desktop frame.
const ComposeFab = defineAsyncComponent(() => import('@/components/layout/ComposeFab.vue'));
const MobileTabBar = defineAsyncComponent(() => import('@/components/layout/MobileTabBar.vue'));
const MailboxSwitcherSheet = defineAsyncComponent(() => import('@/components/layout/MailboxSwitcherSheet.vue'));
const ConnectionBanner = defineAsyncComponent(() => import('@/components/layout/ConnectionBanner.vue'));
const ShortcutHelper = defineAsyncComponent(() => import('@/components/navigation/ShortcutHelper.vue'));
const FilePickerOverlay = defineAsyncComponent(() => import('@/components/mail/FilePickerOverlay.vue'));
const OnboardingFlow = defineAsyncComponent(() => import('@/components/onboarding/OnboardingFlow.vue'));
const SettingsModal = defineAsyncComponent(() => import('@/components/settings/SettingsModal.vue'));
// The shared yes/no and one-line dialogs: their state lives in `useConfirm` /
// `usePrompt`, so the frame (ModalShell, focus trap) loads with the first question.
const ConfirmDialog = defineAsyncComponent(() => import('@/components/ui/ConfirmDialog.vue'));
const PromptDialog = defineAsyncComponent(() => import('@/components/ui/PromptDialog.vue'));

watch(() => route.fullPath, () => {
  if (ui.mailboxSwitcherOpen) {
    ui.closeMailboxSwitcher();
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
    :class="['shell', { 'shell--mobile': ui.isMobile, 'shell--tab-bar': tabBarVisible, 'shell--composer-standalone': standaloneCompose }]"
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

      <!-- The phone has no sidebar and so no sync button. Keep connection
           status visible here, with resting state suppressed. -->
      <SyncStatusLine
        v-if="ui.isMobile && !standaloneCompose"
        class="mobile-sync-status"
        hide-idle
        banner-failures
      />

      <main>
        <RouterView />
      </main>

      <!-- Phone: the four areas. In the flow, so the list ends at its top
           edge instead of running underneath it. -->
      <MobileTabBar v-if="tabBarVisible" />
    </div>

    <ComposerHost v-if="composer.isOpen" />

    <!-- Writing is an action, not a place: a button above the tab bar, not a tab. -->
    <ComposeFab
      v-if="ui.isMobile && !standaloneCompose && !COMPOSE_FAB_HIDDEN_ON.has(String(route.name))"
    />

    <!-- Phone: lists, folders and accounts, opened from the mail list header. -->
    <MailboxSwitcherSheet v-if="ui.isMobile && mailboxSwitcherMounted" />

    <OmniSearch v-if="ui.omniOpen" />
    <ShortcutHelper v-if="ui.shortcutsOpen" />
    <FilePickerOverlay v-if="topicsStore.pickerTarget" />
    <SettingsModal v-if="ui.settingsOpen" />
    <OnboardingFlow v-if="!settingsStore.onboardingComplete || ui.accountSetupOpen" />
    <Transition name="key-sequence">
      <div v-if="ui.keySequenceHint" class="key-sequence-hint" role="status">
        <kbd>g</kbd>
        <span>{{ ui.keySequenceHint }}</span>
      </div>
    </Transition>
    <ToastStack v-if="toastMounted" />
    <ConfirmDialog v-if="confirmState.pending" />
    <PromptDialog v-if="promptState.pending" />
    <UndoFab v-if="toastMounted" :variant="ui.isSinglePane ? 'floating' : 'docked'" />
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
  /* No sidebar here for the receipts to step past. */
  --toast-inset-start: 0px;
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

/* Phone with the tab bar: it sits on the bottom safe area and pads itself,
   so the shell stops short of it. Fixed bottom-lane surfaces (compose FAB,
   undo, toasts) read the inset to stay clear of it. */
.shell--tab-bar {
  padding-bottom: 0;
  --satchel-tab-bar-inset: var(--tab-bar-height);
}
</style>
