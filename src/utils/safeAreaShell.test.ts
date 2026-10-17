import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

describe('safe-area shell contract', () => {
  it('opts the webview into device cutout insets', () => {
    const html = readFileSync(new URL('../../index.html', import.meta.url), 'utf8');
    expect(html).toMatch(/name="viewport"[^>]*viewport-fit=cover/);
  });

  it('defines all four safe-area edges once with zero fallbacks', () => {
    const base = readFileSync(new URL('../styles/base.css', import.meta.url), 'utf8');
    for (const edge of ['top', 'right', 'bottom', 'left']) {
      expect(base).toContain(
        `--satchel-safe-area-${edge}: env(safe-area-inset-${edge}, 0px)`,
      );
    }
  });

  it('separates responsive pane mode from native window chrome', () => {
    const app = readFileSync(new URL('../App.vue', import.meta.url), 'utf8');
    const sidebar = readFileSync(
      new URL('../components/layout/Sidebar.vue', import.meta.url),
      'utf8',
    );

    expect(app).toContain('window-drag-strip desktop-window-chrome');
    expect(app).not.toContain('mobile-titlebar');
    expect(app).not.toContain('safe-area-inset-top, 38px');
    expect(sidebar).toContain('desktop-titlebar desktop-window-chrome');
    expect(sidebar).toContain('flex h-full flex-col');
    expect(sidebar).not.toContain('grid-rows-[auto_auto_1fr_auto_auto]');
  });
});
