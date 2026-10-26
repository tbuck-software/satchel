// @vitest-environment jsdom
import { nextTick } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setPersistenceStorageProvider, type StorageLike } from '@/persistence';
import { layoutModeForWidth, useUiStore } from './ui';

beforeEach(() => {
  setActivePinia(createPinia());
  window.localStorage.clear();
});

afterEach(() => {
  window.localStorage.clear();
});

describe('responsive layout modes', () => {
  it.each([
    [390, { isMobile: true, isSinglePane: true, isTablet: false }],
    [768, { isMobile: false, isSinglePane: true, isTablet: true }],
    [1020, { isMobile: false, isSinglePane: true, isTablet: true }],
    [1021, { isMobile: false, isSinglePane: true, isTablet: true }],
    [1030, { isMobile: false, isSinglePane: false, isTablet: false }],
  ])('maps width %d to the matching pane mode', (width, expected) => {
    expect(layoutModeForWidth(width)).toEqual(expected);
  });

  it('accounts for saved pane widths when deciding whether the reader fits', () => {
    expect(layoutModeForWidth(1100, 420, 760).isSinglePane).toBe(true);
    expect(layoutModeForWidth(1562, 420, 760).isSinglePane).toBe(false);
  });

  it('updates the pane mode on resize and while dragging either divider', async () => {
    const previousWidth = window.innerWidth;
    Object.defineProperty(window, 'innerWidth', { value: 1440, configurable: true });
    const ui = useUiStore();
    try {
      expect(ui.isSinglePane).toBe(false);
      ui.setSidebarWidth(420);
      ui.setThreadListWidth(760);
      await nextTick();
      expect(ui.isSinglePane).toBe(true);
      Object.defineProperty(window, 'innerWidth', { value: 1600, configurable: true });
      window.dispatchEvent(new Event('resize'));
      expect(ui.isSinglePane).toBe(false);
    } finally {
      ui.$dispose();
      Object.defineProperty(window, 'innerWidth', { value: previousWidth, configurable: true });
    }
  });
});

describe('layout persistence', () => {
  it('uses the stable defaults when no widths are stored', () => {
    const ui = useUiStore();

    expect(ui.sidebarWidth).toBe(240);
    expect(ui.threadListWidth).toBe(408);
  });

  it('clamps saved widths to the same limits as resizing', () => {
    window.localStorage.setItem('satchel.ui.sidebarWidth', '100');
    window.localStorage.setItem('satchel.ui.threadListWidth', '9999.5');

    const ui = useUiStore();

    expect(ui.sidebarWidth).toBe(220);
    expect(ui.threadListWidth).toBe(760);
  });

  it('keeps Number legacy semantics while falling back for non-finite values', () => {
    window.localStorage.setItem('satchel.ui.sidebarWidth', '');
    window.localStorage.setItem('satchel.ui.threadListWidth', 'Infinity');

    const ui = useUiStore();

    expect(ui.sidebarWidth).toBe(220);
    expect(ui.threadListWidth).toBe(408);
  });

  it('starts the new-sender review collapsed and remembers a toggle', () => {
    const ui = useUiStore();
    expect(ui.newSendersExpanded).toBe(false);

    ui.toggleNewSendersExpanded();

    expect(ui.newSendersExpanded).toBe(true);
    expect(window.localStorage.getItem('satchel.ui.newSendersExpanded')).toBe('true');
    ui.$dispose();

    setActivePinia(createPinia());
    expect(useUiStore().newSendersExpanded).toBe(true);
  });

  it('clamps and rounds only in setters before persisting', async () => {
    const ui = useUiStore();

    ui.setSidebarWidth(219.6);
    ui.setThreadListWidth(760.6);
    await nextTick();

    expect(ui.sidebarWidth).toBe(220);
    expect(ui.threadListWidth).toBe(760);
    expect(window.localStorage.getItem('satchel.ui.sidebarWidth')).toBe('220');
    expect(window.localStorage.getItem('satchel.ui.threadListWidth')).toBe('760');

    ui.setSidebarWidth(420.6);
    ui.setThreadListWidth(319.6);
    await nextTick();

    expect(ui.sidebarWidth).toBe(420);
    expect(ui.threadListWidth).toBe(320);
  });

  it('keeps store creation and watcher writes safe when storage throws', async () => {
    const setItem = vi.fn(() => {
      throw new Error('set denied');
    });
    const throwingStorage: StorageLike = {
      getItem: () => {
        throw new Error('get denied');
      },
      setItem,
      removeItem: () => {
        throw new Error('remove denied');
      },
    };
    const restoreProvider = setPersistenceStorageProvider(() => throwingStorage);

    try {
      const ui = useUiStore();

      expect(ui.sidebarWidth).toBe(240);
      expect(ui.threadListWidth).toBe(408);
      expect(() => ui.setSidebarWidth(300)).not.toThrow();
      expect(() => ui.setThreadListWidth(500)).not.toThrow();
      await expect(nextTick()).resolves.toBeUndefined();
      expect(setItem).toHaveBeenCalledTimes(2);
    } finally {
      restoreProvider();
    }
  });
});
