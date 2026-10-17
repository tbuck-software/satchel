// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';

// The policy state is a module-level singleton loaded from localStorage at
// import time; a fresh module per test keeps the tests independent.
async function freshPolicy() {
  vi.resetModules();
  const mod = await import('./useImagePolicy');
  return mod.useImagePolicy();
}

beforeEach(() => {
  localStorage.clear();
});

describe('useImagePolicy allow-list', () => {
  it('allows an exact sender, case- and whitespace-insensitively', async () => {
    const policy = await freshPolicy();
    policy.allow(' Alice@Example.COM ');
    expect(policy.isAllowed('alice@example.com')).toBe(true);
    expect(policy.isAllowed('bob@example.com')).toBe(false);
  });

  it('requires a trusted DMARC pass for an allowed domain', async () => {
    const policy = await freshPolicy();
    policy.allowDomain('@Example.COM');
    expect(policy.isAllowed('newsletter@example.com')).toBe(false);
    expect(policy.isAllowed('newsletter@example.com', false)).toBe(false);
    expect(policy.isAllowed('newsletter@example.com', true)).toBe(true);
    expect(policy.isAllowed('billing@example.com', true)).toBe(true);
    expect(policy.isAllowed('news@other.example')).toBe(false);
  });

  it('does not treat a subdomain as the allowed domain', async () => {
    const policy = await freshPolicy();
    policy.allowDomain('example.com');
    expect(policy.isAllowed('news@mail.example.com')).toBe(false);
  });

  it('ignores empty domains and senders without a domain', async () => {
    const policy = await freshPolicy();
    policy.allowDomain('  @ ');
    expect(policy.isAllowed('anyone@anywhere.example')).toBe(false);
    expect(policy.isAllowed('no-at-sign')).toBe(false);
  });

  it('persists senders and domains across module reloads', async () => {
    let policy = await freshPolicy();
    policy.allow('Alice@Example.com');
    policy.allowDomain('shop.example');

    policy = await freshPolicy();
    expect(policy.isAllowed('alice@example.com')).toBe(true);
    expect(policy.isAllowed('anyone@shop.example', true)).toBe(true);
  });

  it('allows everything in the "all" mode, and persists the mode', async () => {
    let policy = await freshPolicy();
    expect(policy.state.mode).toBe('allowlist');
    expect(policy.isAllowed('anyone@anywhere.example')).toBe(false);
    policy.setMode('all');
    expect(policy.isAllowed('anyone@anywhere.example')).toBe(true);
    expect(policy.isAllowed(null)).toBe(true);

    policy = await freshPolicy();
    expect(policy.state.mode).toBe('all');
    policy.setMode('allowlist');
    expect(policy.isAllowed('anyone@anywhere.example')).toBe(false);
  });

  it('leaves the allow-list decisive in the "accepted" mode', async () => {
    // The accepted-sender lookup lives with the caller; for the policy module
    // itself the mode behaves exactly like the allow-list mode.
    const policy = await freshPolicy();
    policy.setMode('accepted');
    expect(policy.isAllowed('anyone@anywhere.example')).toBe(false);
    policy.allow('anyone@anywhere.example');
    expect(policy.isAllowed('anyone@anywhere.example')).toBe(true);
  });

  it('migrates the legacy allow-all flag into the mode', async () => {
    localStorage.setItem('satchel:images-allow-all', 'true');
    let policy = await freshPolicy();
    expect(policy.state.mode).toBe('all');

    // Choosing any mode rewrites the new key and drops the legacy one.
    policy.setMode('accepted');
    expect(localStorage.getItem('satchel:images-allow-all')).toBeNull();
    policy = await freshPolicy();
    expect(policy.state.mode).toBe('accepted');
  });

  it('revokes senders and domains independently', async () => {
    const policy = await freshPolicy();
    policy.allow('alice@example.com');
    policy.allowDomain('example.com');

    policy.revoke('alice@example.com');
    // Still allowed via the authenticated domain.
    expect(policy.isAllowed('alice@example.com', true)).toBe(true);

    policy.revokeDomain('@example.com');
    expect(policy.isAllowed('alice@example.com')).toBe(false);
  });
});
