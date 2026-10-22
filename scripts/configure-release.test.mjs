import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const script = fileURLToPath(new URL('./configure-release.mjs', import.meta.url));

describe('release preparation on native checkouts', () => {
  for (const newline of ['\n', '\r\n']) {
    for (const channel of ['stable', 'nightly']) {
      it(`prepares ${channel} with ${newline === '\n' ? 'LF' : 'CRLF'} files`, () => {
        const root = mkdtempSync(join(tmpdir(), 'satchel-configure-release-'));
        try {
          for (const folder of ['src-tauri', 'output', 'plugins/example']) {
            mkdirSync(join(root, folder), { recursive: true });
          }
          const version = channel === 'stable' ? '0.1.1' : '0.1.1-nightly.42';
          writeFileSync(join(root, 'package.json'), JSON.stringify({ version: '0.1.0' }));
          writeFileSync(join(root, 'src-tauri/Cargo.toml'), '[package]\nname = "satchel"\nversion = "0.1.0"\n'.replace(/\n/g, newline));
          writeFileSync(join(root, 'Cargo.lock'), '[[package]]\nname = "satchel"\nversion = "0.1.0"\n'.replace(/\n/g, newline));
          writeFileSync(join(root, 'output/release-plan.json'), JSON.stringify({ version, channel }));
          writeFileSync(join(root, 'src-tauri/tauri.conf.json'), JSON.stringify({ app: { windows: [{ label: 'main', create: false }] } }));
          writeFileSync(join(root, 'src-tauri/tauri.daily.conf.json'), JSON.stringify({ productName: 'Satchel Daily', app: { windows: [{ title: 'Satchel Daily' }] } }));

          execFileSync(process.execPath, [script], { cwd: root, stdio: 'pipe' });

          expect(JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version).toBe(version);
          expect(readFileSync(join(root, 'src-tauri/Cargo.toml'), 'utf8')).toContain(`version = "${version}"`);
          expect(readFileSync(join(root, 'Cargo.lock'), 'utf8')).toBe(`[[package]]${newline}name = "satchel"${newline}version = "${version}"${newline}`);
          const config = JSON.parse(readFileSync(join(root, 'output/release-config.json'), 'utf8'));
          expect(config.version).toBe(version);
          expect(config.bundle.resources['../output/bundled-plugins/example/dist/component.wasm']).toBe('plugins/example/dist/component.wasm');
          expect(config.bundle.resources['../output/bundled-plugins/example/Cargo.toml']).toBe('plugins/example/Cargo.toml');
          expect(Object.keys(config.bundle.resources).some(path => path.startsWith('../plugins/'))).toBe(false);
          expect(config.productName).toBe('Satchel');
          expect(config.identifier).toBe('de.tbuck.satchel');
          expect(config.app).toBeUndefined();
        } finally {
          rmSync(root, { recursive: true, force: true });
        }
      });
    }
  }

  it('keeps the installed Tauri JavaScript API compatible with the Rust runtime', () => {
    const lock = readFileSync(new URL('../Cargo.lock', import.meta.url), 'utf8');
    const rust = /name = "tauri"\r?\nversion = "(\d+\.\d+)\.\d+"/.exec(lock)?.[1];
    const api = JSON.parse(readFileSync(new URL('../node_modules/@tauri-apps/api/package.json', import.meta.url), 'utf8'));
    expect(rust).toBeDefined();
    expect(api.version.split('.').slice(0, 2).join('.')).toBe(rust);
  });
});
