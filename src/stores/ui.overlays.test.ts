import { beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useUiStore } from './ui';

beforeEach(() => {
  setActivePinia(createPinia());
});

describe('dismissing the topmost overlay', () => {
  it('reports that nothing was open', () => {
    expect(useUiStore().dismissTopmostOverlay()).toBe(false);
  });

  it('closes settings', () => {
    const ui = useUiStore();
    ui.openSettings();

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.settingsOpen).toBe(false);
  });

  /**
   * Account setup opens *from* settings. Backing out of it has to land on
   * settings again — skipping straight to the mailbox loses the place the user
   * was working in.
   */
  it('returns from account setup to settings rather than past it', () => {
    const ui = useUiStore();
    ui.openSettings();
    ui.openAccountSetup();
    expect(ui.settingsOpen).toBe(false);

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.accountSetupOpen).toBe(false);
    expect(ui.settingsOpen).toBe(true);

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.settingsOpen).toBe(false);
  });

  /**
   * Settings are reached *from* the mobile drawer, so the drawer has served its
   * purpose by then. Leaving it open only meant finding it again underneath
   * once settings closed.
   */
  it('closes the mobile sidebar when settings open from it', () => {
    const ui = useUiStore();
    ui.openMobileSidebar();
    ui.openSettings();

    expect(ui.mobileSidebarOpen).toBe(false);

    ui.dismissTopmostOverlay();
    expect(ui.settingsOpen).toBe(false);
    expect(ui.dismissTopmostOverlay()).toBe(false);
  });

  it('closes the mobile sidebar on its own', () => {
    const ui = useUiStore();
    ui.openMobileSidebar();

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.mobileSidebarOpen).toBe(false);
  });

  /**
   * Single pane, the settings detail is a screen of its own: back returns to
   * the section list first, and only the next one leaves settings. With both
   * panes visible there is nothing to step back from.
   */
  it('leaves the settings detail before leaving settings when single-pane', () => {
    const ui = useUiStore();
    ui.isSinglePane = true;
    ui.openSettings();
    ui.openSettingsDetail();

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.settingsDetailOpen).toBe(false);
    expect(ui.settingsOpen).toBe(true);

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.settingsOpen).toBe(false);
  });

  it('closes settings in one step when both panes fit', () => {
    const ui = useUiStore();
    ui.isSinglePane = false;
    ui.openSettings();
    ui.openSettingsDetail();

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.settingsOpen).toBe(false);
  });

  it('jumps straight to the section a caller asked for', () => {
    const ui = useUiStore();
    ui.openSettings('signatures');
    expect(ui.settingsDetailOpen).toBe(true);
  });

  it('closes the omni panel and restores the input mode', () => {
    const ui = useUiStore();
    ui.openOmni();
    expect(ui.inputMode).toBe('insert');

    expect(ui.dismissTopmostOverlay()).toBe(true);
    expect(ui.omniOpen).toBe(false);
    expect(ui.inputMode).toBe('normal');
  });
});
