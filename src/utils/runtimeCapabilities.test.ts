import { describe, expect, it } from 'vitest';
import { supportsDesktopBackgroundControls } from './runtimeCapabilities';

describe('runtime capability UI gates', () => {
  it('hides tray and autostart settings from mobile runtimes', () => {
    expect(supportsDesktopBackgroundControls({ tray: false, autostart: false })).toBe(false);
  });

  it('shows the settings when the native runtime supports either integration', () => {
    expect(supportsDesktopBackgroundControls({ tray: true, autostart: false })).toBe(true);
    expect(supportsDesktopBackgroundControls({ tray: false, autostart: true })).toBe(true);
  });

});
