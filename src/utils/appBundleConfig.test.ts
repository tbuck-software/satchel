import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

describe('desktop app bundle configuration', () => {
  it('keeps local installation app-only without narrowing release bundles', () => {
    const packageManifest = JSON.parse(
      readFileSync(new URL('../../package.json', import.meta.url), 'utf8'),
    ) as { scripts?: Record<string, string> };
    const installer = readFileSync(
      new URL('../../scripts/install-mac.mjs', import.meta.url),
      'utf8',
    );

    expect(packageManifest.scripts?.['tauri:install']).toContain('--bundles app');
    expect(packageManifest.scripts?.['tauri:build']).not.toContain('--bundles');
    expect(packageManifest.scripts?.['release:mac']).toContain('npm run tauri:build');
    expect(installer).toMatch(/stable:\s*{\s*buildScript: 'tauri:install'/);
  });

  it('pins the Tauri app to the desktop binary when auxiliary binaries exist', () => {
    const tauriConfig = JSON.parse(
      readFileSync(new URL('../../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
    ) as {
      mainBinaryName?: string;
      app?: { windows?: Array<{ hiddenTitle?: boolean }> };
    };
    const cargoManifest = readFileSync(
      new URL('../../src-tauri/Cargo.toml', import.meta.url),
      'utf8',
    );

    expect(tauriConfig.mainBinaryName).toBe('satchel');
    expect(tauriConfig.app?.windows?.[0]?.hiddenTitle).toBe(true);
    expect(cargoManifest).toMatch(/^default-run\s*=\s*"satchel"$/m);
  });

  it('allows custom titlebar regions to move desktop windows', () => {
    const capability = JSON.parse(
      readFileSync(
        new URL('../../src-tauri/capabilities/default.json', import.meta.url),
        'utf8',
      ),
    ) as { permissions?: string[] };
    const sidebar = readFileSync(
      new URL('../components/layout/Sidebar.vue', import.meta.url),
      'utf8',
    );

    // The call itself moved behind `services/appWindow`, so the chain is
    // checked in both halves: the sidebar reaches the wrapper, the wrapper
    // reaches the window.
    const appWindow = readFileSync(
      new URL('../services/appWindow.ts', import.meta.url),
      'utf8',
    );

    expect(capability.permissions).toContain('core:window:allow-start-dragging');
    expect(sidebar).toMatch(
      /<header[^>]*data-tauri-drag-region[^>]*@mousedown="startWindowDrag"/,
    );
    expect(sidebar).toContain("from '@/services/appWindow'");
    expect(appWindow).toContain('getCurrentWindow().startDragging()');
  });
});
