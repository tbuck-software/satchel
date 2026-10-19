import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const installMac = readFileSync(
  fileURLToPath(new URL('./install-mac.mjs', import.meta.url)),
  'utf8',
);
const importDaily = readFileSync(new URL('./import-daily-data.mjs', import.meta.url), 'utf8');
const releaseCheck = readFileSync(
  fileURLToPath(new URL('./release-check.mjs', import.meta.url)),
  'utf8',
);
const vendorFonts = readFileSync(
  fileURLToPath(new URL('./vendor-fonts.mjs', import.meta.url)),
  'utf8',
);
const makefile = readFileSync(
  fileURLToPath(new URL('../Makefile', import.meta.url)),
  'utf8',
);

describe('Make install targets', () => {
  it('accepts both install-all spellings without installing macOS twice', () => {
    expect(makefile).toMatch(/^all: install-all$/m);
    expect(makefile).toMatch(/^install-all: _install-android-after-mac$/m);
    expect(makefile).toMatch(/^_install-android-after-mac: install$/m);
  });
});

describe('install-mac script', () => {
  it('backs up app data before anything replaces the installed app', () => {
    const backupAt = installMac.indexOf('backupAppData();');
    const replaceAt = installMac.indexOf('renameSync(stagedApp, channel.targetApp);');
    expect(backupAt).toBeGreaterThan(-1);
    expect(replaceAt).toBeGreaterThan(backupAt);
    // The running app is quit before the backup, so the SQLite backup API
    // never reads a database that is being written concurrently.
    expect(installerCall('quitRunningApp();', 1)).toBeLessThan(backupAt);
    /** @param {string} needle @param {number} occurrence */
    function installerCall(needle, occurrence) {
      let index = -1;
      for (let i = 0; i < occurrence; i += 1) {
        index = installMac.indexOf(needle, index + 1);
      }
      return index;
    }
  });

  it('preserves the keychain-only key in automatic backups and cleans it up when pruning', () => {
    expect(installMac).toContain('copyProfileMasterKey(appSupportRoot, backupDir, { backup: true })');
    expect(installMac.indexOf('removeBackupMasterKey(join(backupsRoot, stale))'))
      .toBeLessThan(installMac.indexOf('rmSync(join(backupsRoot, stale)'));
    expect(installMac).not.toContain('falling back to a raw file copy');
  });

  it('verifies the imported key before replacing Daily data and preserves the previous key', () => {
    expect(importDaily).toContain('copyProfileMasterKey(dailyRoot, backupDir, { backup: true })');
    const transfer = importDaily.indexOf('copyProfileMasterKey(stableRoot, dailyRoot)');
    expect(transfer).toBeGreaterThan(importDaily.indexOf('backupDailyData();'));
    expect(transfer).toBeLessThan(importDaily.indexOf('renameSync(tempDb, dailyDb)'));
    expect(importDaily).not.toContain('Daily can reuse existing Keychain entries');
    expect(importDaily).toContain("filter: (source) => source !== join(stableRoot, 'crypto', 'master.key')");
    expect(importDaily).toContain("filter: (source) => source !== join(dailyRoot, 'crypto', 'master.key')");
  });

  it('keeps a rollback path when replacing the bundle fails mid-way', () => {
    expect(installMac).toContain('renameSync(previousApp, channel.targetApp)');
  });

  it('copies the local .env with owner-only permissions into the channel AppData', () => {
    expect(installMac).toContain('chmodSync(appSupportEnv, 0o600)');
  });

  it('invokes pnpm, not npm, for the build step', () => {
    expect(installMac).toContain("run('pnpm', ['run', channel.buildScript]);");
    expect(installMac).not.toContain("run('npm'");
  });
});

describe('release-check script', () => {
  it('runs the complete release gate for both Rust crates and the production frontend', () => {
    for (const command of [
      "['run', 'i18n:check']",
      "['exec', 'vue-tsc', '-b']",
      "['fmt', '--all', '--check']",
      "['clippy', '-p', 'satchel', '-p', 'satchel-core', '--all-targets', '--', '-D', 'warnings']",
      "['test', '-p', 'satchel', '-p', 'satchel-core']",
      "['run', 'plugins:build']",
      "['run', 'build']",
      "['run', 'bundle:check']",
      "['run', 'test:smoke']",
      "['run', 'test:csp']",
    ]) expect(releaseCheck).toContain(command);
    expect(releaseCheck).toContain("SATCHEL_REQUIRE_PLUGIN_ARTIFACTS: '1'");
    expect(releaseCheck).toContain("SATCHEL_REQUIRE_GPG: '1'");
    expect(releaseCheck).toContain("run(gpg, ['--version']);");
    expect(releaseCheck).not.toContain("--manifest-path");
  });

  it('uses the lockfile-installed Tauri CLI for every build channel', () => {
    const { scripts } = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
    for (const name of ['tauri:dev', 'tauri:build', 'tauri:install', 'tauri:daily']) {
      expect(scripts[name]).not.toContain('npx');
      expect(scripts[name]).toContain('&& tauri ');
    }
  });

  it('fails fast on a dirty tree unless explicitly overridden', () => {
    expect(releaseCheck).toMatch(/SATCHEL_ALLOW_DIRTY_RELEASE/);
  });

  it('runs the test suite through pnpm', () => {
    expect(releaseCheck).toContain("run('pnpm', ['exec', 'vitest', 'run', '--maxWorkers=3']);");
    expect(releaseCheck).not.toContain("run('npm'");
  });
});

describe('vendor-fonts script', () => {
  it('exits with a clear message instead of an unhandled rejection when offline', () => {
    expect(vendorFonts).toMatch(/Font vendoring failed/);
    expect(vendorFonts).toContain('process.exit(1)');
    // Every fetch in the script goes through the guarded helpers.
    expect(
      vendorFonts.match(/await fetch\(/g)?.length ?? 0,
      'new raw fetch calls must be wrapped in fetchText/fetchBuffer',
    ).toBe(2);
  });
});
