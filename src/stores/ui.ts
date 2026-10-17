import { computed, onScopeDispose, ref, watch } from 'vue';
import { defineStore } from 'pinia';
import { uiPersistence } from '@/persistence/ui';

export type InputMode = 'normal' | 'command' | 'insert';

const MOBILE_BREAKPOINT = 768;
/**
 * The minimum viewport for a split. Wider saved panes can require more room.
 */
const SINGLE_PANE_BREAKPOINT = 1020;
const READER_MIN_WIDTH = 380;
const DIVIDER_WIDTH = 1;
const clampSidebarWidth = (width: number) => Math.min(420, Math.max(220, Math.round(width)));
const clampThreadListWidth = (width: number) => Math.min(760, Math.max(320, Math.round(width)));

export function layoutModeForWidth(width: number, sidebarWidth = 240, threadListWidth = 408) {
  const isMobile = width < MOBILE_BREAKPOINT;
  const availableWidth = width - (isMobile ? 0 : sidebarWidth + DIVIDER_WIDTH);
  const isSinglePane = width <= SINGLE_PANE_BREAKPOINT
    || availableWidth < threadListWidth + DIVIDER_WIDTH + READER_MIN_WIDTH;
  return {
    isMobile,
    isSinglePane,
    isTablet: !isMobile && isSinglePane,
  };
}

export const useUiStore = defineStore('ui', () => {
  const inputMode = ref<InputMode>('normal');
  const omniOpen = ref(false);
  const omniMode = ref<'default' | 'reminder'>('default');
  const omniTargetThreadIds = ref<string[]>([]);
  const keySequenceHint = ref('');
  const shortcutsOpen = ref(false);
  const settingsOpen = ref(false);
  /** Section the settings modal should show when it opens; null = keep current. */
  const settingsSection = ref<string | null>(null);
  /**
   * Whether the settings detail is showing instead of the section list.
   *
   * Only meaningful while single-pane; with both panes on screen the detail is
   * always visible and there is nothing to go back from.
   */
  const settingsDetailOpen = ref(false);
  const accountSetupOpen = ref(false);
  const sidebarWidth = ref(clampSidebarWidth(uiPersistence.sidebarWidth.read()));
  const threadListWidth = ref(clampThreadListWidth(uiPersistence.threadListWidth.read()));
  const initialLayout = layoutModeForWidth(
    typeof window !== 'undefined' ? window.innerWidth : SINGLE_PANE_BREAKPOINT + 1,
    sidebarWidth.value,
    threadListWidth.value,
  );
  const isMobile = ref(initialLayout.isMobile);
  const isSinglePane = ref(initialLayout.isSinglePane);
  const isTablet = ref(initialLayout.isTablet);
  const mobileSidebarOpen = ref(false);

  function updateLayout() {
    if (typeof window !== 'undefined') {
      const layout = layoutModeForWidth(window.innerWidth, sidebarWidth.value, threadListWidth.value);
      isMobile.value = layout.isMobile;
      isSinglePane.value = layout.isSinglePane;
      isTablet.value = layout.isTablet;
      if (!isMobile.value) {
        mobileSidebarOpen.value = false;
      }
    }
  }
  if (typeof window !== 'undefined') {
    window.addEventListener('resize', updateLayout);
    onScopeDispose(() => window.removeEventListener('resize', updateLayout));
  }
  watch([sidebarWidth, threadListWidth], updateLayout, { flush: 'sync' });

  function openOmni() {
    mobileSidebarOpen.value = false;
    omniMode.value = 'default';
    omniTargetThreadIds.value = [];
    omniOpen.value = true;
    inputMode.value = 'insert';
  }

  function openReminder(threadIds: string[]) {
    const ids = Array.from(new Set(threadIds)).filter(Boolean);
    if (ids.length === 0) return;
    omniMode.value = 'reminder';
    omniTargetThreadIds.value = ids;
    omniOpen.value = true;
    inputMode.value = 'insert';
  }

  function leaveOmniStep(): boolean {
    if (!omniOpen.value || omniMode.value === 'default') return false;
    omniMode.value = 'default';
    omniTargetThreadIds.value = [];
    return true;
  }

  function showKeySequenceHint(hint: string) {
    keySequenceHint.value = hint;
  }

  function clearKeySequenceHint() {
    keySequenceHint.value = '';
  }

  function closeOmni() {
    omniOpen.value = false;
    omniMode.value = 'default';
    omniTargetThreadIds.value = [];
    inputMode.value = 'normal';
  }

  function toggleShortcuts() {
    mobileSidebarOpen.value = false;
    shortcutsOpen.value = !shortcutsOpen.value;
  }

  function openSettings(section?: string) {
    // Reached from the mobile sidebar, which is not a route and so would stay
    // open underneath — and be sitting there again once settings close.
    mobileSidebarOpen.value = false;
    settingsSection.value = section ?? null;
    // Jumping straight to a section — "Signatur bearbeiten" from a toast —
    // should land on that section, not on the list it lives in.
    settingsDetailOpen.value = section != null;
    settingsOpen.value = true;
  }

  function openSettingsDetail() {
    settingsDetailOpen.value = true;
  }

  function closeSettings() {
    settingsOpen.value = false;
    settingsDetailOpen.value = false;
  }

  function openAccountSetup() {
    settingsOpen.value = false;
    accountSetupOpen.value = true;
  }

  function closeAccountSetup() {
    accountSetupOpen.value = false;
  }

  function returnToAccountSettings() {
    accountSetupOpen.value = false;
    settingsOpen.value = true;
  }

  /**
   * Close whatever sits on top, and report whether anything was there.
   *
   * The system back gesture asks this first. Without it Tauri falls through to
   * `WebView.canGoBack()`, and since every one of these surfaces is state
   * rather than a route, there is no history entry to return to — back leaves
   * the app instead of leaving the dialog.
   *
   * Order is innermost first: account setup opens *from* settings, so backing
   * out of it must land on settings rather than skip past it.
   */
  /**
   * How many back steps the overlays currently owe.
   *
   * The back handler mirrors this into history entries, so nesting — settings,
   * then a section, then account setup — costs one press per level instead of
   * collapsing to a single one.
   */
  /**
   * Surfaces owned by other stores that the back gesture should also close.
   *
   * Registered rather than imported: the ui store must not depend on the
   * composer or topics stores just to know something is on screen, and a
   * surface that forgets to register simply keeps its old behaviour instead of
   * breaking.
   *
   * `isOpen` must read reactive state — the depth below is a computed, so a
   * plain captured boolean would be read once and never again.
   */
  const externalOverlays = ref<
    Array<{ id: number; isOpen: () => boolean; dismiss: () => void }>
  >([]);
  const backNavigations = ref<
    Array<{ id: number; isOpen: () => boolean; dismiss: () => void }>
  >([]);
  let nextOverlayId = 0;

  function registerOverlay(entry: { isOpen: () => boolean; dismiss: () => void }) {
    // Identified by id rather than by reference: what goes into the ref comes
    // back out as a reactive proxy, so `item !== entry` never matches and the
    // entry would outlive the component that registered it.
    const id = (nextOverlayId += 1);
    externalOverlays.value.push({ id, ...entry });
    return () => {
      externalOverlays.value = externalOverlays.value.filter((item) => item.id !== id);
    };
  }

  /**
   * Register a state-backed screen below dialogs and the mobile sidebar.
   *
   * Inbox folders and grouped lists are not routes, but on a phone they still
   * behave like navigation destinations. Keeping them separate from overlays
   * makes Back close a drawer or composer first, then leave the folder below.
   */
  function registerBackNavigation(entry: { isOpen: () => boolean; dismiss: () => void }) {
    const id = (nextOverlayId += 1);
    backNavigations.value.push({ id, ...entry });
    return () => {
      backNavigations.value = backNavigations.value.filter((item) => item.id !== id);
    };
  }

  const overlayDepth = computed(() => {
    let depth = externalOverlays.value.filter((entry) => entry.isOpen()).length;
    depth += backNavigations.value.filter((entry) => entry.isOpen()).length;
    if (mobileSidebarOpen.value) depth += 1;
    if (omniOpen.value) depth += 1;
    if (shortcutsOpen.value) depth += 1;
    if (settingsOpen.value) depth += 1;
    if (settingsOpen.value && isSinglePane.value && settingsDetailOpen.value) depth += 1;
    if (accountSetupOpen.value) depth += 1;
    return depth;
  });

  /**
   * Something covers the page: a dialog, menu, sheet, the palette or settings.
   * Unlike `overlayDepth` this leaves out back navigations, which are screens.
   */
  const modalOpen = computed(() =>
    overlayDepth.value > backNavigations.value.filter((entry) => entry.isOpen()).length,
  );

  function dismissTopmostOverlay(): boolean {
    // Registered surfaces sit above the built-in ones: a composer or a picker
    // is opened *from* whatever is underneath it.
    const external = externalOverlays.value.filter((entry) => entry.isOpen());
    const topExternal = external.at(-1);
    if (topExternal) {
      topExternal.dismiss();
      return true;
    }
    // Single pane: the settings detail *is* a screen of its own, so leaving it
    // returns to the section list rather than closing settings outright.
    if (settingsOpen.value && isSinglePane.value && settingsDetailOpen.value) {
      settingsDetailOpen.value = false;
      return true;
    }
    if (accountSetupOpen.value) {
      returnToAccountSettings();
      return true;
    }
    if (settingsOpen.value) {
      closeSettings();
      return true;
    }
    if (shortcutsOpen.value) {
      shortcutsOpen.value = false;
      return true;
    }
    if (omniOpen.value) {
      if (leaveOmniStep()) return true;
      closeOmni();
      return true;
    }
    if (mobileSidebarOpen.value) {
      closeMobileSidebar();
      return true;
    }
    const navigations = backNavigations.value.filter((entry) => entry.isOpen());
    const topNavigation = navigations.at(-1);
    if (topNavigation) {
      topNavigation.dismiss();
      return true;
    }
    return false;
  }

  function openMobileSidebar() {
    mobileSidebarOpen.value = true;
  }

  function closeMobileSidebar() {
    mobileSidebarOpen.value = false;
  }

  function toggleMobileSidebar() {
    mobileSidebarOpen.value = !mobileSidebarOpen.value;
  }

  function setSidebarWidth(width: number) {
    sidebarWidth.value = clampSidebarWidth(width);
  }

  function setThreadListWidth(width: number) {
    threadListWidth.value = clampThreadListWidth(width);
  }

  watch(sidebarWidth, (value) => {
    uiPersistence.sidebarWidth.write(value);
  });

  watch(threadListWidth, (value) => {
    uiPersistence.threadListWidth.write(value);
  });

  return {
    inputMode,
    omniOpen,
    omniMode,
    omniTargetThreadIds,
    keySequenceHint,
    shortcutsOpen,
    settingsOpen,
    settingsSection,
    accountSetupOpen,
    sidebarWidth,
    threadListWidth,
    openOmni,
    openReminder,
    leaveOmniStep,
    showKeySequenceHint,
    clearKeySequenceHint,
    closeOmni,
    toggleShortcuts,
    openSettings,
    closeSettings,
    openAccountSetup,
    closeAccountSetup,
    returnToAccountSettings,
    isMobile,
    isSinglePane,
    isTablet,
    dismissTopmostOverlay,
    overlayDepth,
    modalOpen,
    registerOverlay,
    registerBackNavigation,
    settingsDetailOpen,
    openSettingsDetail,
    mobileSidebarOpen,
    setSidebarWidth,
    setThreadListWidth,
    openMobileSidebar,
    closeMobileSidebar,
    toggleMobileSidebar,
  };
});
