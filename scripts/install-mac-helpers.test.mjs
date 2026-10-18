import { describe, expect, it } from 'vitest';
import { processListContainsApp, stableInstallProblem, versionTagProblem } from './install-mac-helpers.mjs';

describe('processListContainsApp', () => {
  it('recognizes a translocated app without confusing stable, daily, or helper processes', () => {
    const root = '/private/var/folders/ab/example/T/AppTranslocation/UUID/d/';
    const process = `${root}Satchel.app/Contents/MacOS/satchel --example`;
    expect(processListContainsApp(process, '/Applications/Satchel.app', 'satchel')).toBe(true);
    expect(processListContainsApp(process, '/Applications/Satchel Daily.app', 'satchel')).toBe(false);
    expect(processListContainsApp(`${root}Satchel.app/Contents/MacOS/satchel-helper`, '/Applications/Satchel.app', 'satchel')).toBe(false);
    expect(processListContainsApp(`/bin/cat ${process}`, '/Applications/Satchel.app', 'satchel')).toBe(false);
  });
  const app = '/Applications/Satchel Daily.app';

  it('recognizes the desktop process even when the app path contains spaces', () => {
    expect(
      processListContainsApp(
        `123 helper\n${app}/Contents/MacOS/satchel\n`,
        app,
        'satchel',
      ),
    ).toBe(true);
  });

  it('does not confuse a sibling helper binary with the desktop process', () => {
    expect(
      processListContainsApp(`${app}/Contents/MacOS/satchel-helper\n`, app, 'satchel'),
    ).toBe(false);
  });
});

describe('stable install guard', () => {
  const clean = { channel: 'stable', branch: 'main', status: '', allowUnstable: false };
  it('accepts clean main and an isolated daily channel', () => {
    expect(stableInstallProblem(clean)).toBeNull();
    expect(stableInstallProblem({ ...clean, channel: 'daily', branch: 'feature', status: ' M x' })).toBeNull();
  });
  it('rejects feature branches, detached checkouts and dirty main', () => {
    for (const state of [{ branch: 'fix/topic' }, { branch: 'HEAD' }, { status: ' M x' }, { status: '?? new' }]) {
      expect(stableInstallProblem({ ...clean, ...state })).toContain('clean main');
    }
  });
  it('requires the explicit override for an experimental stable install', () => {
    expect(stableInstallProblem({ ...clean, branch: 'fix/topic', status: ' M x', allowUnstable: true })).toBeNull();
  });
});

describe('stable version tags', () => {
  it('allows a new version and repeated installation of the same commit', () => {
    expect(versionTagProblem('v0.1.0', '', 'current')).toBeNull();
    expect(versionTagProblem('v0.1.0', 'current', 'current')).toBeNull();
  });
  it('refuses to move an existing release tag', () => {
    expect(versionTagProblem('v0.1.0', 'previous', 'current')).toContain('Bump the release version');
  });
});
