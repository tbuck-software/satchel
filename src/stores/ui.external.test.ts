import { ref } from 'vue';
import { beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useUiStore } from './ui';

beforeEach(() => {
  setActivePinia(createPinia());
});

describe('overlays owned by other stores', () => {
  /**
   * `isOpen` has to read reactive state — the depth is a computed, so a plain
   * boolean would be read once and cached. Every real caller reads a store, so
   * the tests use a ref for the same reason.
   */
  it('counts a registered surface towards the back depth', () => {
    const ui = useUiStore();
    const open = ref(false);
    ui.registerOverlay({ isOpen: () => open.value, dismiss: () => { open.value = false; } });

    expect(ui.overlayDepth).toBe(0);
    open.value = true;
    expect(ui.overlayDepth).toBe(1);
  });

  /**
   * A composer or picker is opened *from* whatever is beneath it, so back has
   * to leave it first — otherwise the surface underneath closes and the one in
   * front of the user survives.
   */
  it('dismisses a registered surface before the built-in ones', () => {
    const ui = useUiStore();
    const composerOpen = ref(true);
    ui.registerOverlay({ isOpen: () => composerOpen.value, dismiss: () => { composerOpen.value = false; } });
    ui.openSettings();

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(composerOpen.value).toBe(false);
    expect(ui.settingsOpen).toBe(true);

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.settingsOpen).toBe(false);
  });

  it('stops counting a surface once it unregisters', () => {
    const ui = useUiStore();
    const open = ref(true);
    const unregister = ui.registerOverlay({ isOpen: () => open.value, dismiss: () => {} });

    expect(ui.overlayDepth).toBe(1);
    unregister();
    expect(ui.overlayDepth).toBe(0);
  });

  it('reports nothing to dismiss when a registered surface is closed', () => {
    const ui = useUiStore();
    ui.registerOverlay({ isOpen: () => false, dismiss: () => {} });
    expect(ui.dismissTopmostOverlay()).toBe(false);
  });
});
