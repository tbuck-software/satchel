// @vitest-environment jsdom
import { defineComponent } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useToast } from './useToast';

const harness = vi.hoisted(() => ({
  route: { name: 'inbox' as string },
  settings: { singleKeyShortcuts: true, keybindings: {} as Record<string, string> },
  ui: {
    omniOpen: false,
    shortcutsOpen: false,
    modalOpen: false,
    inputMode: 'normal',
    openOmni: vi.fn(),
    toggleShortcuts: vi.fn(),
    dismissTopmostOverlay: vi.fn(() => false),
    showKeySequenceHint: vi.fn(),
    clearKeySequenceHint: vi.fn(),
    openReminder: vi.fn(),
  },
  threads: {
    selectedThreadIds: [] as string[],
    activeThreadId: 'thread-1',
    find: () => ({ id: 'thread-1', unreadCount: 0, isMuted: false }),
    nextThread: vi.fn(),
    prevThread: vi.fn(),
    toggleSelectionForThread: vi.fn(),
    selectAllVisibleThreads: vi.fn(),
    clearSelection: vi.fn(),
  },
  actions: {
    archive: vi.fn(),
    trash: vi.fn(),
    setRead: vi.fn(),
    setAside: vi.fn(),
    waitForReply: vi.fn(),
    moveToInbox: vi.fn(),
    togglePinned: vi.fn(),
    setMuted: vi.fn(),
    markSpam: vi.fn(),
  },
  topics: { openPicker: vi.fn() },
  router: { push: vi.fn(), replace: vi.fn(), back: vi.fn() },
  composer: { isOpen: false, openNew: vi.fn(), openReply: vi.fn(), openForward: vi.fn() },
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({
    ...harness.router,
    currentRoute: { get value() { return harness.route; } },
  }),
}));
vi.mock('@/stores/ui', () => ({ useUiStore: () => harness.ui }));
vi.mock('@/stores/settings', () => ({ useSettingsStore: () => harness.settings }));
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => harness.threads }));
vi.mock('@/stores/topics', () => ({ useTopicsStore: () => harness.topics }));
vi.mock('@/stores/composer', () => ({ useComposerStore: () => harness.composer }));
vi.mock('./useThreadActions', () => ({ useThreadActions: () => harness.actions }));
vi.mock('./useListKeyboardNavigation', () => ({
  moveListKeyboardFocus: () => false,
  openListKeyboardFocus: () => false,
  leaveListKeyboardLevel: () => false,
}));
vi.mock('./useThreadMessageKeyboardNavigation', () => ({
  moveThreadMessageFocus: () => false,
  openThreadMessageFocus: () => false,
  toggleAllThreadMessages: () => false,
  toggleFocusedThreadMessage: () => false,
}));

import { useKeyboard } from './useKeyboard';
import { registerSettingsFilterField } from './settingsFilterFocus';

let mounted: ReturnType<typeof mount> | null = null;

function mountKeyboard() {
  mounted = mount(defineComponent({
    setup: () => {
      useKeyboard();
      return () => null;
    },
  }));
}

function press(key: string, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
  document.body.dispatchEvent(event);
  return event;
}

/** The keys that act on the inbox's active thread or move through the list. */
function pressMailKeys() {
  return ['e', '#', 'Backspace', 'Delete', 'ArrowDown', 'ArrowUp', 'j', 't', 'Enter', 'l']
    .map((key) => press(key));
}

function expectNoMailAction() {
  expect(harness.actions.archive).not.toHaveBeenCalled();
  expect(harness.actions.trash).not.toHaveBeenCalled();
  expect(harness.threads.nextThread).not.toHaveBeenCalled();
  expect(harness.threads.prevThread).not.toHaveBeenCalled();
  expect(harness.topics.openPicker).not.toHaveBeenCalled();
  expect(harness.router.push).not.toHaveBeenCalled();
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.route = { name: 'inbox' };
  harness.settings.singleKeyShortcuts = true;
  harness.settings.keybindings = {};
  harness.ui.omniOpen = false;
  harness.ui.shortcutsOpen = false;
  harness.ui.modalOpen = false;
  harness.composer.isOpen = false;
});

afterEach(() => {
  mounted?.unmount();
  mounted = null;
  useToast().clear();
});

describe('shortcut context', () => {
  it.each(['settings', 'calendar', 'contacts', 'drafts', 'search'])(
    'leaves the inbox thread alone on %s',
    (route) => {
      harness.route = { name: route };
      mountKeyboard();

      const events = pressMailKeys();

      expectNoMailAction();
      // Arrow keys and Enter keep their default: scrolling, activating.
      expect(events.some((event) => event.defaultPrevented)).toBe(false);
    },
  );

  it('still navigates and opens the palette from those pages', () => {
    harness.route = { name: 'settings' };
    mountKeyboard();

    press(':');
    press('g');
    press('i');
    press('n');

    expect(harness.ui.openOmni).toHaveBeenCalledTimes(1);
    expect(harness.router.push).toHaveBeenCalledWith({ name: 'inbox' });
    expect(harness.composer.openNew).toHaveBeenCalledTimes(1);
  });

  it('acts on the active thread in the inbox and the reader', () => {
    mountKeyboard();
    press('e');
    harness.route = { name: 'thread' };
    press('#');

    expect(harness.actions.archive).toHaveBeenCalledWith(['thread-1']);
    expect(harness.actions.trash).toHaveBeenCalledWith(['thread-1']);
  });

  it('does nothing behind a dialog, menu or the help', () => {
    harness.ui.modalOpen = true;
    mountKeyboard();

    pressMailKeys();
    press('n');
    press('g');
    press('i');
    press(':');

    expectNoMailAction();
    expect(harness.composer.openNew).not.toHaveBeenCalled();
    expect(harness.ui.openOmni).not.toHaveBeenCalled();
  });

  it('still dismisses the dialog with Escape', () => {
    harness.ui.modalOpen = true;
    harness.ui.dismissTopmostOverlay.mockReturnValueOnce(true);
    mountKeyboard();

    press('Escape');

    expect(harness.ui.dismissTopmostOverlay).toHaveBeenCalledTimes(1);
    expect(harness.router.back).not.toHaveBeenCalled();
  });

  it('closes the help with its own key', () => {
    harness.ui.modalOpen = true;
    harness.ui.shortcutsOpen = true;
    mountKeyboard();

    press('?');

    expect(harness.ui.toggleShortcuts).toHaveBeenCalledTimes(1);
  });

  it('lets the calendar have t', () => {
    harness.route = { name: 'calendar' };
    mountKeyboard();

    const event = press('t');

    expect(event.defaultPrevented).toBe(false);
    expect(harness.topics.openPicker).not.toHaveBeenCalled();
  });
});

describe('single-key switch per binding', () => {
  it('keeps an action rebound to a chord working with the switch off', () => {
    harness.settings.singleKeyShortcuts = false;
    harness.settings.keybindings = { archive: 'mod+e' };
    mountKeyboard();

    press('e');
    press('e', { metaKey: true });

    expect(harness.actions.archive).toHaveBeenCalledTimes(1);
  });

  it('keeps the arrows and Delete with the switch off', () => {
    harness.settings.singleKeyShortcuts = false;
    mountKeyboard();

    press('ArrowDown');
    press('Delete');

    expect(harness.threads.nextThread).toHaveBeenCalledTimes(1);
    expect(harness.actions.trash).toHaveBeenCalledTimes(1);
  });
});

describe('undo key', () => {
  function receipt(handler: () => void) {
    useToast().success('Marked as done', {
      action: { label: 'Undo', handler, present: 'fab' },
    });
  }

  it('runs the newest receipt with z and with Cmd+Z', async () => {
    const first = vi.fn();
    const second = vi.fn();
    receipt(first);
    receipt(second);
    mountKeyboard();

    press('z');
    expect(second).toHaveBeenCalledTimes(1);
    expect(first).not.toHaveBeenCalled();
    // A receipt is dismissed once its handler has accepted the undo.
    await flushPromises();

    press('z', { metaKey: true });
    expect(first).toHaveBeenCalledTimes(1);
  });

  it('leaves Cmd+Z to the browser when there is nothing to undo', () => {
    mountKeyboard();

    expect(press('z', { metaKey: true }).defaultPrevented).toBe(false);
  });

  it('keeps Cmd+Z inside a text field', () => {
    const handler = vi.fn();
    receipt(handler);
    mountKeyboard();
    const input = document.createElement('input');
    document.body.append(input);

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'z', metaKey: true, bubbles: true }));

    expect(handler).not.toHaveBeenCalled();
    input.remove();
  });
});

describe('focus after activation', () => {
  function clickButton(detail: number) {
    const button = document.createElement('button');
    document.body.append(button);
    button.focus();
    button.dispatchEvent(new MouseEvent('click', { bubbles: true, detail }));
    const focused = document.activeElement === button;
    button.remove();
    return focused;
  }

  it('drops focus from a button clicked with the pointer', () => {
    mountKeyboard();
    expect(clickButton(1)).toBe(false);
  });

  /** Enter or Space fire a click with detail 0; the next Tab must continue from here. */
  it('keeps focus on a button activated from the keyboard', () => {
    mountKeyboard();
    expect(clickButton(0)).toBe(true);
  });
});

// #164: Esc did nothing in Settings on the desktop, and there was no filter to reach.
describe('Settings keys', () => {
  beforeEach(() => {
    harness.route = { name: 'settings' };
  });

  it('focuses the section filter with / instead of opening the palette', () => {
    const focus = vi.fn();
    const unregister = registerSettingsFilterField(focus);
    mountKeyboard();

    press('/');

    expect(focus).toHaveBeenCalledTimes(1);
    expect(harness.ui.openOmni).not.toHaveBeenCalled();
    unregister();
  });

  it('opens the palette with / when no filter is on screen', () => {
    mountKeyboard();

    press('/');

    expect(harness.ui.openOmni).toHaveBeenCalledTimes(1);
  });

  it('leaves Settings with Escape', () => {
    mountKeyboard();

    press('Escape');

    // jsdom starts without history, so leaving means the inbox.
    expect(harness.router.push).toHaveBeenCalledWith({ name: 'inbox' });
  });

  it('only leaves a field on the first Escape, so typed text is not dropped with the page', () => {
    const input = document.createElement('input');
    document.body.append(input);
    input.focus();
    mountKeyboard();

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));

    expect(document.activeElement).not.toBe(input);
    expect(harness.router.push).not.toHaveBeenCalled();
    expect(harness.router.back).not.toHaveBeenCalled();
    input.remove();
  });

  it('leaves Settings from an empty filter field with one Escape', () => {
    const filter = document.createElement('input');
    filter.setAttribute('data-settings-filter', '');
    document.body.append(filter);
    filter.focus();
    mountKeyboard();

    filter.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));

    expect(harness.router.push).toHaveBeenCalledWith({ name: 'inbox' });
    filter.remove();
  });
});
