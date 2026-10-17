import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { compareReleaseVersions, releaseVersion, verifyUpdate } from './release-model.mjs';

const fixture = JSON.parse(readFileSync(new URL('../tests/fixtures/updater-signature.json', import.meta.url), 'utf8'));

describe('release channel ordering', () => {
  it('makes nightlies of the next stable version without changing source versions', () => {
    expect(releaseVersion('0.1.0', 'v0.1.7', 'nightly', 42)).toBe('0.1.8-nightly.42');
    expect(releaseVersion('0.1.0', 'v0.1.7', 'stable', 43)).toBe('0.1.8');
    expect(releaseVersion('0.2.0', 'v0.1.7', 'stable', 44)).toBe('0.2.1');
  });

  it('handles the first release and rejects invalid versions and Windows overflow', () => {
    expect(releaseVersion('0.1.0', null, 'stable', 1)).toBe('0.1.1');
    expect(() => releaseVersion('0.1.0', 'nightly', 'stable', 2)).toThrow();
    expect(() => releaseVersion('0.1.65535', null, 'stable', 2)).toThrow();
    expect(() => releaseVersion('0.1.0', null, 'nightly', 0)).toThrow();
  });

  it('orders nightlies numerically and puts their stable release after them', () => {
    expect(compareReleaseVersions('0.1.2-nightly.10', '0.1.2-nightly.9')).toBe(1);
    expect(compareReleaseVersions('0.1.2', '0.1.2-nightly.10')).toBe(1);
    expect(compareReleaseVersions('0.1.1', '0.1.2-nightly.1')).toBe(-1);
    expect(compareReleaseVersions('0.1.2', '0.1.2')).toBe(0);
  });
});

describe('installer verification against the native Tauri signer', () => {
  it('accepts the Tauri CLI signature and rejects changed archive bytes', () => {
    expect(() => verifyUpdate(Buffer.from(fixture.payload), fixture.signature, fixture.publicKey)).not.toThrow();
    expect(() => verifyUpdate(Buffer.from(fixture.payload + 'tampered'), fixture.signature, fixture.publicKey)).toThrow('verification failed');
  });

  it('rejects another key and a changed trusted comment', () => {
    const decoded = Buffer.from(fixture.publicKey, 'base64').toString('utf8').split('\n');
    const key = Buffer.from(decoded[1], 'base64');
    key[10] ^= 1;
    decoded[1] = key.toString('base64');
    const wrongKey = Buffer.from(decoded.join('\n')).toString('base64');
    expect(() => verifyUpdate(Buffer.from(fixture.payload), fixture.signature, wrongKey)).toThrow();
    const lines = Buffer.from(fixture.signature, 'base64').toString('utf8').split('\n');
    lines[2] += ' changed';
    const signature = lines.join('\n');
    expect(() => verifyUpdate(Buffer.from(fixture.payload), Buffer.from(signature).toString('base64'), fixture.publicKey)).toThrow();
  });
});
