import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { buildEmailCsp } from './emailCsp';

/**
 * These tests exist because the CSP was silently removed once (commit aa03d78)
 * during a renderer refactor, leaving the tracking protection on the regex
 * layer alone for ~10 days. The last two tests assert the wiring, not just the
 * builder — a correct builder nobody calls is exactly the failure mode we hit.
 */
describe('buildEmailCsp', () => {
  it('locks everything down by default', () => {
    const csp = buildEmailCsp(false);
    expect(csp).toContain("default-src 'none'");
    expect(csp).toContain("object-src 'none'");
    expect(csp).toContain("form-action 'none'");
    expect(csp).toContain("base-uri 'none'");
  });

  it('never allows scripts, regardless of the image policy', () => {
    for (const allowRemote of [false, true]) {
      const csp = buildEmailCsp(allowRemote);
      expect(csp).not.toContain('script-src');
      expect(csp).not.toContain("'unsafe-eval'");
    }
  });

  function sources(csp: string, directive: string): string[] {
    const part = csp.split('; ').find((entry) => entry.startsWith(`${directive} `));
    return part ? part.split(' ').slice(1) : [];
  }

  it('blocks remote images when the sender is not trusted', () => {
    const imgSrc = sources(buildEmailCsp(false), 'img-src');
    expect(imgSrc.length).toBeGreaterThan(0);
    // The wildcard schemes are what let tracking pixels through. Host-scoped
    // sources like https://asset.localhost are local Tauri assets and fine.
    expect(imgSrc).not.toContain('https:');
    expect(imgSrc).not.toContain('http:');
    // Inline/local resources must keep working — these are not tracking pixels.
    expect(imgSrc).toContain('data:');
    expect(imgSrc).toContain('cid:');
    expect(imgSrc).toContain('blob:');
  });

  it('allows remote images only once the sender is trusted', () => {
    const imgSrc = sources(buildEmailCsp(true), 'img-src');
    expect(imgSrc).toContain('https:');
    expect(imgSrc).toContain('http:');
  });

  it('never widens media-src beyond the image policy', () => {
    expect(sources(buildEmailCsp(false), 'media-src')).not.toContain('https:');
    expect(sources(buildEmailCsp(true), 'media-src')).toContain('https:');
  });

  it('keeps inline styles working (mail relies on them heavily)', () => {
    expect(buildEmailCsp(false)).toContain("style-src 'unsafe-inline'");
  });
});

describe('CSP wiring', () => {
  function read(relativePath: string): string {
    return readFileSync(fileURLToPath(new URL(relativePath, import.meta.url)), 'utf8');
  }

  it('IsolatedEmailDocument injects the policy into the srcdoc head', () => {
    const source = read('../components/mail/IsolatedEmailDocument.vue');
    expect(source).toContain('buildEmailCsp');
    expect(source).toMatch(/http-equiv="Content-Security-Policy"/);
    // Must come before any other head content, or it does not cover it.
    const cspIndex = source.indexOf('http-equiv="Content-Security-Policy"');
    const charsetIndex = source.indexOf('<meta charset="utf-8">');
    expect(cspIndex).toBeGreaterThan(-1);
    expect(charsetIndex).toBeGreaterThan(-1);
    expect(cspIndex).toBeLessThan(charsetIndex);
  });

  it('sets a cross-platform sans fallback for mail without a surviving font', () => {
    const source = read('../components/mail/IsolatedEmailDocument.vue');
    expect(source).toMatch(/\.satchel-email-body\s*\{[^}]*font-family:[^}]*sans-serif/s);
  });

  it('the iframe stays sandboxed without allow-scripts', () => {
    const source = read('../components/mail/IsolatedEmailDocument.vue');
    const sandboxAttributes = source.match(/sandbox="[^"]*"/g) ?? [];
    expect(sandboxAttributes.length).toBeGreaterThan(0);
    // Asserted on the attributes rather than on the file text, so the comments
    // explaining what the scriptless frame costs may name the flag. WebKit runs
    // no listener inside such a frame, which is why link interaction lives
    // outside it — that reasoning has to stay readable in the source.
    for (const attribute of sandboxAttributes) {
      expect(attribute).not.toContain('allow-scripts');
    }
    // Nor may anything widen the sandbox at runtime.
    expect(source).not.toMatch(/setAttribute\(\s*['"]sandbox['"]/);
    expect(source).not.toMatch(/\.sandbox\s*=/);
  });

  it('EmailRenderer passes the per-sender image policy to every mail document', () => {
    const source = read('../components/mail/EmailRenderer.vue');
    const isolatedUsages = source.match(/<IsolatedEmailDocument/g) ?? [];
    const markdownUsages = source.match(/<SafeMarkdownBody/g) ?? [];
    const wired = source.match(/:allow-remote-images="imagesEffectivelyAllowed"/g) ?? [];
    expect(isolatedUsages.length).toBeGreaterThan(0);
    expect(markdownUsages.length).toBeGreaterThan(0);
    expect(wired.length).toBe(isolatedUsages.length + markdownUsages.length);
  });

  it('the Tauri webview ships a CSP', () => {
    const config = JSON.parse(read('../../src-tauri/tauri.conf.json'));
    const csp = config?.app?.security?.csp;
    expect(csp, 'tauri.conf.json app.security.csp must not be null').toBeTruthy();
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("object-src 'none'");
  });
});
