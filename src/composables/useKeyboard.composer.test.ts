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
    dismissTopmostOverlay: vi.fn(() => true),
    showKeySequenceHint: vi.fn(),
    clearKeySequenceHint: vi.fn(),
    openReminder: vi.fn(),
  },
  threads: {
    selectedThreadIds: [] as string[],
    activeThreadId: 'thread-1',
    threads: [{ id: 'thread-1', unreadCount: 1 }],
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

let mounted: ReturnType<typeof mount> | null = null;

function mountKeyboard() {
  mounted = mount(defineComponent({
    setup: () => {
      useKeyboard();
      return () => null;
    },
  }));
  return mounted;
}

function press(key: string, init: KeyboardEventInit = {}) {
  document.body.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...init }));
}

describe('shortcuts while the composer is open', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    harness.settings.singleKeyShortcuts = true;
    harness.ui.omniOpen = false;
    harness.composer.isOpen = false;
    document.body.innerHTML = '';
  });

  afterEach(() => {
    mounted?.unmount();
    mounted = null;
  });

  it('touches no thread while the composer has the keyboard', () => {
    harness.composer.isOpen = true;
    mountKeyboard();

    press('e');
    press('#');
    press('u');
    press('Backspace');
    press('Delete');

    expect(harness.actions.archive).not.toHaveBeenCalled();
    expect(harness.actions.trash).not.toHaveBeenCalled();
    expect(harness.actions.setRead).not.toHaveBeenCalled();
  });

  it('lets Escape and the palette combo through', () => {
    harness.composer.isOpen = true;
    mountKeyboard();

    press('Escape');
    press('k', { metaKey: true });

    expect(harness.ui.dismissTopmostOverlay).toHaveBeenCalledTimes(1);
    expect(harness.ui.openOmni).toHaveBeenCalledTimes(1);
  });

  it('still trashes with Delete once the composer is closed', () => {
    mountKeyboard();

    press('Backspace');

    expect(harness.actions.trash).toHaveBeenCalledWith(['thread-1']);
  });

  it('leaves focus inside the composer after a toolbar click', () => {
    mountKeyboard();
    document.body.innerHTML = `
      <div data-composer-surface><button id="bold" type="button">B</button></div>
      <button id="outside" type="button">O</button>
    `;

    const bold = document.getElementById('bold') as HTMLButtonElement;
    bold.focus();
    bold.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));

    expect(document.activeElement).toBe(bold);
  });

  it('still parks focus off a button everywhere else', () => {
    mountKeyboard();
    document.body.innerHTML = '<button id="outside" type="button">O</button>';

    const outside = document.getElementById('outside') as HTMLButtonElement;
    outside.focus();
    // A pointer click; keyboard activation (detail 0) keeps the focus.
    outside.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));

    expect(document.activeElement).toBe(document.body);
  });
});
