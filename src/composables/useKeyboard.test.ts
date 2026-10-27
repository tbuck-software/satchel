// @vitest-environment jsdom
import { defineComponent } from 'vue';
import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  settings: { singleKeyShortcuts: true, keybindings: {} as Record<string, string> },
  ui: {
    omniOpen: false,
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
    threads: [{ id: 'thread-1', unreadCount: 0 }],
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
  },
  composer: { isOpen: false, openNew: vi.fn(), openReply: vi.fn(), openForward: vi.fn() },
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({
    push: vi.fn(),
    replace: vi.fn(),
    back: vi.fn(),
    currentRoute: { value: { name: 'inbox' } },
  }),
}));
vi.mock('@/stores/ui', () => ({ useUiStore: () => harness.ui }));
vi.mock('@/stores/settings', () => ({ useSettingsStore: () => harness.settings }));
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => harness.threads }));
vi.mock('@/stores/topics', () => ({ useTopicsStore: () => ({ openPicker: vi.fn() }) }));
vi.mock('@/stores/composer', () => ({ useComposerStore: () => harness.composer }));
vi.mock('./useThreadActions', () => ({ useThreadActions: () => harness.actions }));
// No list or reader is mounted in this test, so nothing owns the focus.
vi.mock('./useListKeyboardNavigation', () => ({
  moveListKeyboardFocus: () => false,
  openListKeyboardFocus: () => false,
  leaveListKeyboardLevel: () => false,
  listKeyboardTargets: () => null,
  listKeyboardSharesSelection: () => false,
}));
vi.mock('./useThreadMessageKeyboardNavigation', () => ({
  moveThreadMessageFocus: () => false,
  openThreadMessageFocus: () => false,
  toggleAllThreadMessages: () => false,
  toggleFocusedThreadMessage: () => false,
}));

import { useKeyboard } from './useKeyboard';

let mounted: ReturnType<typeof mount> | null = null;

// The listeners live on `window`; a leaked instance would keep handling keys
// in the next test and count every press twice.
function mountKeyboard() {
  mounted = mount(defineComponent({
    setup: () => {
      useKeyboard();
      return () => null;
    },
  }));
  return mounted;
}

/** Dispatched on the body, the way a real keystroke reaches the window. */
function press(key: string, init: KeyboardEventInit = {}) {
  document.body.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }));
}

describe('single-key shortcut switch', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.settings.singleKeyShortcuts = true;
    harness.ui.omniOpen = false;
    harness.threads.selectedThreadIds = [];
    harness.threads.activeThreadId = 'thread-1';
  });

  afterEach(() => {
    mounted?.unmount();
    mounted = null;
  });

  it('opens the command palette and the help even with the switch off', () => {
    harness.settings.singleKeyShortcuts = false;
    mountKeyboard();

    press('?');
    press(':');
    press('/');

    expect(harness.ui.toggleShortcuts).toHaveBeenCalledTimes(1);
    expect(harness.ui.openOmni).toHaveBeenCalledTimes(2);
  });

  it('silences the bare action keys when the switch is off', () => {
    harness.settings.singleKeyShortcuts = false;
    mountKeyboard();

    press('j');
    press('e');
    press('#');
    press('u');

    expect(harness.threads.nextThread).not.toHaveBeenCalled();
    expect(harness.actions.archive).not.toHaveBeenCalled();
    expect(harness.actions.trash).not.toHaveBeenCalled();
    expect(harness.actions.setRead).not.toHaveBeenCalled();
  });

  it('keeps the bare action keys while the switch is on', () => {
    mountKeyboard();

    press('j');
    press('e');

    expect(harness.threads.nextThread).toHaveBeenCalledTimes(1);
    expect(harness.actions.archive).toHaveBeenCalledWith(['thread-1']);
  });

  /** `e` is the second Done key; a binding saved for it keeps its id. */
  it('marks done from either Done key, including a rebound second key', () => {
    harness.settings.keybindings = { archive: 'y' };
    mountKeyboard();

    press('d');
    press('y');
    press('e');

    expect(harness.actions.archive).toHaveBeenCalledTimes(2);
    expect(harness.actions.archive).toHaveBeenCalledWith(['thread-1']);
    harness.settings.keybindings = {};
  });

  it('still refuses to steal a system combo', () => {
    mountKeyboard();

    press('/', { metaKey: true });

    expect(harness.ui.openOmni).not.toHaveBeenCalled();
  });
});
