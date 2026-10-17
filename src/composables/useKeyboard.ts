import { onMounted, onUnmounted } from 'vue';
import { useRouter } from 'vue-router';
import { useUiStore } from '@/stores/ui';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore } from '@/stores/threads';
import { useTopicsStore } from '@/stores/topics';
import { useComposerStore } from '@/stores/composer';
import { useThreadActions } from './useThreadActions';
import { useMessageLinks } from './useMessageLinks';
import { useUndoFab } from './useUndoFab';
import { activeThreadReaderCommands } from './useThreadReaderCommands';
import { focusSettingsFilter } from './settingsFilterFocus';
import { t } from '@/i18n/runtime';
import {
  bindingFor,
  bindingStrokes,
  keybindingContextActive,
  keybindingEnabled,
  keyboardEventStroke,
  matchesKeybinding,
  type ShortcutScope,
} from '@/config/keybindings';
import { KEYBINDING_SPECS, type KeybindingContext } from '@/config/keybindingDefaults';
import {
  leaveListKeyboardLevel,
  moveListKeyboardFocus,
  openListKeyboardFocus,
} from './useListKeyboardNavigation';
import {
  moveThreadMessageFocus,
  openThreadMessageFocus,
  toggleAllThreadMessages,
  toggleFocusedThreadMessage,
} from './useThreadMessageKeyboardNavigation';

/**
 * Targets that own their keystrokes. Note that `button` is deliberately NOT in
 * this list: a focused button used to silence every shortcut until the user
 * clicked elsewhere, which broke the mixed mouse/keyboard flow. Buttons are
 * instead blurred after activation (see `blurActivatedButton`), and Space/Enter
 * on a focused button is handled below so it still activates the button.
 */
function isEditableTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName.toLowerCase();
  return tag === 'input' || tag === 'textarea' || tag === 'select' || el.isContentEditable;
}

const CONTEXT_BY_ACTION: ReadonlyMap<string, KeybindingContext> = new Map(
  KEYBINDING_SPECS.map((spec) => [spec.action, spec.context]),
);

/** True while focus sits on a button/link that should keep Enter and Space. */
function isActivatableTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName.toLowerCase();
  return tag === 'button' || tag === 'a' || el.getAttribute('role') === 'button';
}

export function useKeyboard() {
  const router = useRouter();
  const ui = useUiStore();
  const settings = useSettingsStore();
  const threads = useThreadsStore();
  const topics = useTopicsStore();
  const composer = useComposerStore();
  const actions = useThreadActions();
  const messageLinks = useMessageLinks();
  const undoFab = useUndoFab();
  let pendingSequence = '';
  let sequenceTimer: number | null = null;
  // Read once per keystroke; every binding below is judged against it.
  let scope: ShortcutScope = { route: null, covered: false };

  function targetIds(): string[] {
    if (threads.selectedThreadIds.length > 0) {
      return [...threads.selectedThreadIds];
    }
    return threads.activeThreadId ? [threads.activeThreadId] : [];
  }

  function keyFor(action: string): string {
    return bindingFor(action, settings.keybindings);
  }

  function contextActive(context: KeybindingContext): boolean {
    return keybindingContextActive(context, scope);
  }

  /** The `when` clause: the action's context holds and the single-key switch allows its key. */
  function available(action: string): boolean {
    const context = CONTEXT_BY_ACTION.get(action);
    return context !== undefined
      && contextActive(context)
      && keybindingEnabled(action, keyFor(action), settings.singleKeyShortcuts);
  }

  function matches(event: KeyboardEvent, action: string): boolean {
    return available(action) && matchesKeybinding(event, keyFor(action));
  }

  function matchesRangeNavigation(event: KeyboardEvent, action: string): boolean {
    if (!available(action)) return false;
    if (matches(event, action)) return true;
    const binding = keyFor(action);
    return event.shiftKey
      && !event.metaKey
      && !event.ctrlKey
      && !event.altKey
      && binding.length === 1
      && binding === binding.toLocaleLowerCase()
      && event.key.toLocaleLowerCase() === binding;
  }

  function clearSequence() {
    pendingSequence = '';
    ui.clearKeySequenceHint();
    if (sequenceTimer !== null) window.clearTimeout(sequenceTimer);
    sequenceTimer = null;
  }

  function goTo(name: 'inbox' | 'drafts' | 'outbox' | 'calendar' | 'followups' | 'contacts') {
    clearSequence();
    void router.push({ name });
  }

  function handleNavigationSequence(event: KeyboardEvent): boolean {
    const destinations = [
      { action: 'goInbox', route: 'inbox' as const, label: t('shortcuts.sequence.inbox') },
      { action: 'goDrafts', route: 'drafts' as const, label: t('shortcuts.sequence.drafts') },
      { action: 'goOutbox', route: 'outbox' as const, label: t('shortcuts.sequence.outbox') },
      { action: 'goCalendar', route: 'calendar' as const, label: t('shortcuts.sequence.calendar') },
      { action: 'goFollowUps', route: 'followups' as const, label: t('shortcuts.sequence.followUps') },
      { action: 'goContacts', route: 'contacts' as const, label: t('shortcuts.sequence.contacts') },
    ].filter((destination) => available(destination.action));
    const stroke = keyboardEventStroke(event);

    if (pendingSequence) {
      const candidate = `${pendingSequence} ${stroke}`;
      const match = destinations.find(
        (destination) => keyFor(destination.action) === candidate,
      );
      if (match) {
        goTo(match.route);
        return true;
      }
      clearSequence();
    }

    const isPrefix = destinations.some((destination) =>
      bindingStrokes(keyFor(destination.action))[0] === stroke,
    );
    if (!isPrefix) return false;

    pendingSequence = stroke;
    ui.showKeySequenceHint(destinations.map((destination) => destination.label).join(' · '));
    // A chord must not stay armed after the user has mentally moved on. This
    // window is deliberately longer than a double-click but shorter than the
    // transient hint, making both fast and deliberate two-key input reliable.
    sequenceTimer = window.setTimeout(clearSequence, 1_200);
    return true;
  }

  function moveThread(delta: 1 | -1, extendSelection = false) {
    if (moveListKeyboardFocus(delta, extendSelection)) return;

    if (delta > 0) threads.nextThread();
    else threads.prevThread();

    // A routed detail has no list controller mounted. Keep j/k useful there
    // by moving the route with the active thread instead of changing invisible
    // store state behind the reader.
    if (router.currentRoute.value.name === 'thread' && threads.activeThreadId) {
      void router.replace({ name: 'thread', params: { id: threads.activeThreadId } });
    }
  }

  function leaveCurrentLevel(): boolean {
    if (leaveListKeyboardLevel()) return true;
    // Settings is a page with no other way out on the desktop than the sidebar.
    const route = router.currentRoute.value.name;
    if (route !== 'thread' && route !== 'settings') return false;

    if (window.history.length > 1) router.back();
    else void router.push({ name: 'inbox' });
    return true;
  }

  function undoLatest(): boolean {
    const entry = undoFab.entries.value.at(-1);
    if (!entry) return false;
    void undoFab.run(entry);
    return true;
  }

  function handleEscape(target: EventTarget | null) {
    clearSequence();
    if (!ui.dismissTopmostOverlay()) {
      // A field in Settings may hold unsaved text: the first Escape only leaves the field.
      if (
        router.currentRoute.value.name === 'settings'
        && isEditableTarget(target)
        && !(target as HTMLElement).hasAttribute('data-settings-filter')
      ) {
        (target as HTMLElement).blur();
      } else {
        leaveCurrentLevel();
      }
    }
    ui.inputMode = ui.omniOpen ? 'insert' : 'normal';
  }

  /**
   * Fixed keys every mail client has. They ignore the single-key switch but
   * not the context: arrows and Delete belong to a mail list, not to Settings.
   * Returns true when the event was consumed.
   *
   * The command palette and the shortcut help live here: they are how someone
   * who switched the bare keys off finds every action again, and the help
   * itself lists them unconditionally.
   */
  function handleUniversalKeys(event: KeyboardEvent): boolean {
    const stroke = keyboardEventStroke(event);

    if (stroke === 'mod+n' && contextActive('global')) {
      composer.openNew();
      return true;
    }

    if (stroke === 'mod+z' && contextActive('global')) {
      return undoLatest();
    }

    // Everything below is a bare key: never steal system/browser combos.
    if (event.metaKey || event.ctrlKey || event.altKey) return false;

    // In Settings, `/` filters the sections instead.
    if (event.key === '/' && scope.route === 'settings' && !scope.covered && focusSettingsFilter()) {
      return true;
    }

    if (matches(event, 'openCommandPalette') || (event.key === '/' && contextActive('global'))) {
      ui.openOmni();
      return true;
    }

    // The help covers the page itself; its own key has to close it again.
    if (matches(event, 'help') || (ui.shortcutsOpen && matchesKeybinding(event, keyFor('help')))) {
      ui.toggleShortcuts();
      return true;
    }

    if (contextActive('mailbox')) {
      if (event.key === 'ArrowDown') {
        moveThread(1, event.shiftKey);
        return true;
      }

      if (event.key === 'ArrowUp') {
        moveThread(-1, event.shiftKey);
        return true;
      }

      if (event.key === 'Enter') {
        if (router.currentRoute.value.name === 'thread' && openThreadMessageFocus()) return true;
        if (openListKeyboardFocus()) return true;
        if (!threads.activeThreadId) return false;
        void router.push({ name: 'thread', params: { id: threads.activeThreadId } });
        return true;
      }
    }

    if (contextActive('mail') && (event.key === 'Delete' || event.key === 'Backspace')) {
      const ids = targetIds();
      if (ids.length === 0) return false;
      void actions.trash(ids);
      return true;
    }

    return false;
  }

  /** Rebindable actions; each one passes its own context and switch check. */
  function handleBoundKeys(event: KeyboardEvent): boolean {
    if (handleNavigationSequence(event)) return true;

    if (matchesRangeNavigation(event, 'nextThread')) {
      if (router.currentRoute.value.name === 'thread') {
        if (event.shiftKey) moveThread(1);
        else moveThreadMessageFocus(1);
        return true;
      }
      moveThread(1, event.shiftKey);
      return true;
    }

    if (matchesRangeNavigation(event, 'prevThread')) {
      if (router.currentRoute.value.name === 'thread') {
        if (event.shiftKey) moveThread(-1);
        else moveThreadMessageFocus(-1);
        return true;
      }
      moveThread(-1, event.shiftKey);
      return true;
    }

    if (router.currentRoute.value.name === 'thread' && matches(event, 'toggleAllMessages')) {
      return toggleAllThreadMessages();
    }

    if (router.currentRoute.value.name === 'thread' && matches(event, 'toggleMessage')) {
      return toggleFocusedThreadMessage();
    }

    if (matches(event, 'openItem')) {
      if (openListKeyboardFocus()) return true;
      const threadId = threads.activeThreadId;
      if (!threadId) return false;
      void router.push({ name: 'thread', params: { id: threadId } });
      return true;
    }

    if (matches(event, 'leaveItem')) {
      return leaveCurrentLevel();
    }

    const threadId = threads.activeThreadId;

    if (threadId && matches(event, 'toggleSelection')) {
      threads.toggleSelectionForThread(threadId);
      return true;
    }

    if (matches(event, 'selectAll')) {
      threads.selectAllVisibleThreads();
      return true;
    }

    if (matches(event, 'clearSelection')) {
      threads.clearSelection();
      return true;
    }

    if (matches(event, 'reply')) {
      if (!threadId) return false;
      composer.openReply(threadId);
      return true;
    }

    if (matches(event, 'replyAll')) {
      if (!threadId) return false;
      composer.openReply(threadId, true);
      return true;
    }

    if (matches(event, 'forward')) {
      if (!threadId) return false;
      composer.openForward(threadId);
      return true;
    }

    if (matches(event, 'compose')) {
      composer.openNew();
      return true;
    }

    if (matches(event, 'toggleRead')) {
      const ids = targetIds();
      if (ids.length === 0) return false;
      const anyUnread = ids.some((id) => {
        const thread = threads.find(id);
        return (thread?.unreadCount ?? 0) > 0;
      });
      void actions.setRead(ids, anyUnread);
      return true;
    }

    if (matches(event, 'moveWaiting')) {
      void actions.waitForReply(targetIds());
      return true;
    }

    if (matches(event, 'moveSnoozed')) {
      ui.openReminder(targetIds());
      return true;
    }

    if (matches(event, 'markDone')) {
      void actions.archive(targetIds());
      return true;
    }

    if (matches(event, 'moveInbox')) {
      void actions.moveToInbox(targetIds());
      return true;
    }

    if (matches(event, 'setAside')) {
      const ids = targetIds();
      if (ids.length === 0) return false;
      void actions.setAside(ids);
      return true;
    }

    if (matches(event, 'archive')) {
      const ids = targetIds();
      if (ids.length === 0) return false;
      void actions.archive(ids);
      return true;
    }

    if (matches(event, 'trash')) {
      const ids = targetIds();
      if (ids.length === 0) return false;
      void actions.trash(ids);
      return true;
    }

    if (threadId && matches(event, 'pin')) {
      void actions.togglePinned(threadId);
      return true;
    }

    if (matches(event, 'addToTopic')) {
      const ids = targetIds();
      if (ids.length === 0) return false;
      topics.openPicker({ itemType: 'thread', itemIds: ids });
      return true;
    }

    if (threadId && matches(event, 'copyLink')) {
      void messageLinks.copy(threadId);
      return true;
    }

    if (matches(event, 'undo')) {
      return undoLatest();
    }

    if (threadId && matches(event, 'toggleMute')) {
      void actions.setMuted(threadId, !threads.find(threadId)?.isMuted);
      return true;
    }

    if (matches(event, 'markSpam')) {
      const ids = targetIds();
      if (ids.length === 0) return false;
      void actions.markSpam(ids);
      return true;
    }

    if (matches(event, 'print')) {
      const reader = activeThreadReaderCommands();
      if (!reader?.threadId) return false;
      void reader.print('conversation');
      return true;
    }

    return false;
  }

  function onKeyDown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;

    // Let a focused button/link keep its own activation keys.
    if (isActivatableTarget(event.target) && (event.key === 'Enter' || event.key === ' ')) {
      return;
    }

    // Cmd+K / Cmd+F must reach the omni panel even while an input has focus —
    // a quick-access surface that dies the moment you focus a field is useless.
    const mod = event.metaKey || event.ctrlKey;
    const isOmniCombo =
      mod && !event.altKey && (event.key.toLowerCase() === 'k' || event.key.toLowerCase() === 'f');

    if (isEditableTarget(event.target) && event.key !== 'Escape' && !isOmniCombo) {
      return;
    }

    if (event.key === 'Escape') {
      handleEscape(event.target);
      event.preventDefault();
      return;
    }

    scope = {
      route: (router.currentRoute.value.name as string | undefined) ?? null,
      covered: ui.modalOpen,
    };

    // The composer covers the page but is a workspace: the palette stays
    // reachable from it, as it does from the palette itself.
    if (isOmniCombo) {
      if (scope.covered && !composer.isOpen && !ui.omniOpen) return;
      ui.openOmni();
      event.preventDefault();
      return;
    }

    // The composer owns the keyboard while it is open. Its toolbar buttons hand
    // focus back to `<body>`, and without this the next keystroke archived or
    // trashed the thread behind the window.
    if (composer.isOpen) return;

    if (handleUniversalKeys(event) || handleBoundKeys(event)) {
      event.preventDefault();
    }
  }

  /**
   * Keeps focus from parking on a button after a click. Without this, the next
   * bare-key shortcut would land on the button instead of the list.
   *
   * Inside the composer the opposite is true: there the keyboard belongs to the
   * editor, and dropping focus onto `<body>` is how typing after a click on
   * "Bold" ended up in the mailbox instead of the mail.
   */
  function blurActivatedButton(event: MouseEvent) {
    // Enter or Space: the user is on the keyboard and needs the focus to stay.
    if (event.detail === 0) return;
    const el = event.target as HTMLElement | null;
    if (el?.closest?.('[data-composer-surface]')) return;
    const button = el?.closest?.('button');
    if (button && document.activeElement === button) {
      button.blur();
    }
  }

  onMounted(() => {
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('click', blurActivatedButton);
  });

  onUnmounted(() => {
    clearSequence();
    window.removeEventListener('keydown', onKeyDown);
    window.removeEventListener('click', blurActivatedButton);
  });
}
