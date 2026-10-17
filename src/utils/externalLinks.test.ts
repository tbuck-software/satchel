import { beforeAll, describe, expect, it, vi } from 'vitest';
import {
  assessExternalLink,
  linkRegistrableDomain,
  loadPublicSuffixTable,
  parseMailtoLink,
} from './externalLinks';

// The public-suffix table arrives on demand now; every case below asks about a
// registrable domain, so they wait for it once.
beforeAll(() => loadPublicSuffixTable());

/*
 * PERF-08. The table is dynamic, so "not here yet" is a state the assessment
 * has to have an answer for. Fail closed: ask, do not wave through.
 *
 * `tldts` is mocked behind a gate rather than raced against: the module's own
 * `import('tldts')` resolves in a microtask, so a timing-based test would pass
 * for the wrong reason.
 */
describe('before the public-suffix table is loaded', () => {
  it('treats a subdomain of the same registrable domain as a mismatch', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.doMock('tldts', async () => {
      const actual = await vi.importActual<typeof import('tldts')>('tldts');
      await gate;
      return actual;
    });
    vi.resetModules();
    const pending = await import('./externalLinks');

    expect(
      pending.assessExternalLink('https://shop.example.com/x', 'https://example.com'),
    ).toMatchObject({ requiresConfirmation: true, risks: ['display_host_mismatch'] });
    expect(pending.linkRegistrableDomain('shop.example.com')).toBe('shop.example.com');

    release();
    await pending.loadPublicSuffixTable();

    expect(
      pending.assessExternalLink('https://shop.example.com/x', 'https://example.com'),
    ).toMatchObject({ requiresConfirmation: false, risks: [] });
    expect(pending.linkRegistrableDomain('shop.example.com')).toBe('example.com');

    vi.doUnmock('tldts');
    vi.resetModules();
  });
});

describe('external link assessment', () => {
  it('allows matching HTTPS URLs without confirmation', () => {
    expect(assessExternalLink('https://gitlab.code.example/work/3', 'https://gitlab.code.example/work/3')).toMatchObject({
      allowed: true,
      hostname: 'gitlab.code.example',
      requiresConfirmation: false,
      risks: [],
    });
  });

  it('requires confirmation when URL-like visible text points to another host', () => {
    expect(assessExternalLink('https://evil.example/login', 'https://gitlab.code.example')).toMatchObject({
      allowed: true,
      hostname: 'evil.example',
      displayedHostname: 'gitlab.code.example',
      requiresConfirmation: true,
      risks: ['display_host_mismatch'],
    });
  });

  it('also detects a mismatching visible domain without a scheme', () => {
    expect(assessExternalLink('https://evil.example/login', 'gitlab.code.example')).toMatchObject({
      displayedHostname: 'gitlab.code.example',
      requiresConfirmation: true,
      risks: ['display_host_mismatch'],
    });
  });

  it('unwraps a Google outbound redirect before comparing and opening it', () => {
    const href = 'https://www.google.com/url?q=https%3A%2F%2Fgitnrwchat.uni-muenster.de%2Fgitnrw%2Fchannels%2Fap3-4&sa=D&source=calendar';
    expect(assessExternalLink(
      href,
      'https://gitnrwchat.uni-muenster.de/gitnrw/channels/ap3-4',
    )).toMatchObject({
      allowed: true,
      href: 'https://gitnrwchat.uni-muenster.de/gitnrw/channels/ap3-4',
      hostname: 'gitnrwchat.uni-muenster.de',
      displayedHostname: 'gitnrwchat.uni-muenster.de',
      requiresConfirmation: false,
      risks: [],
    });
  });

  it('still warns when the destination inside a Google redirect differs from the label', () => {
    const href = 'https://www.google.com/url?q=https%3A%2F%2Fevil.example%2Flogin';
    expect(assessExternalLink(href, 'https://bank.example/login')).toMatchObject({
      href: 'https://evil.example/login',
      hostname: 'evil.example',
      displayedHostname: 'bank.example',
      requiresConfirmation: true,
      risks: ['display_host_mismatch'],
    });
  });

  it('does not unwrap a Google-shaped redirect on another host', () => {
    const href = 'https://google.com.evil.example/url?q=https%3A%2F%2Fbank.example';
    expect(assessExternalLink(href, 'https://bank.example')).toMatchObject({
      href,
      hostname: 'google.com.evil.example',
      requiresConfirmation: true,
      risks: ['display_host_mismatch'],
    });
  });

  it('compares the registrable domain rather than special-casing www', () => {
    expect(assessExternalLink('https://www.tbuck.de/', 'tbuck.de')).toMatchObject({
      hostname: 'www.tbuck.de',
      displayedHostname: 'tbuck.de',
      requiresConfirmation: false,
      risks: [],
    });
    expect(assessExternalLink(
      'https://shop.example.co.uk/checkout',
      'account.example.co.uk',
    )).toMatchObject({
      requiresConfirmation: false,
      risks: [],
    });
  });

  it('honours private suffixes when comparing independently owned hosts', () => {
    expect(assessExternalLink('https://evil.github.io/login', 'alice.github.io')).toMatchObject({
      requiresConfirmation: true,
      risks: ['display_host_mismatch'],
    });
  });

  it('does not warn for an ordinary descriptive label', () => {
    expect(assessExternalLink('https://gitlab.code.example/work/3', 'Issue ansehen')).toMatchObject({
      allowed: true,
      requiresConfirmation: false,
      risks: [],
    });
  });

  it('reports HTTP without forcing a phishing confirmation', () => {
    expect(assessExternalLink('http://www.example.com/', 'www.example.com')).toMatchObject({
      allowed: true,
      hostname: 'www.example.com',
      requiresConfirmation: false,
      risks: ['insecure_http'],
    });
  });

  it.each([
    ['https://user@example.test', 'embedded_credentials'],
    ['https://127.0.0.1/path', 'ip_address'],
    ['https://xn--pple-43d.example', 'punycode_hostname'],
    ['https://example.test:8443/path', 'unusual_port'],
  ])('flags risky target %s', (href, risk) => {
    const assessment = assessExternalLink(href, 'Open');
    expect(assessment.allowed).toBe(true);
    expect(assessment.requiresConfirmation).toBe(true);
    expect(assessment.risks).toContain(risk);
  });

  it('rejects non-web URL schemes', () => {
    expect(assessExternalLink('javascript:alert(1)', 'Open')).toMatchObject({
      allowed: false,
      requiresConfirmation: false,
    });
  });

  /*
   * #125. Each of these reads as "paypal.com" on screen while the link goes
   * elsewhere; the old check only fired for a label that was exactly an ASCII
   * hostname.
   */
  it.each([
    ['a trailing character a hidden span leaves behind', 'paypal.com!'],
    ['a zero-width space', 'pay\u200Bpal.com'],
    ['a soft hyphen', 'pay\u00ADpal.com'],
    ['a trailing dot', 'paypal.com.'],
    ['fullwidth letters', '\uFF50\uFF41\uFF59\uFF50\uFF41\uFF4C\uFF0E\uFF43\uFF4F\uFF4D'],
    ['an ideographic full stop', 'paypal\u3002com'],
    ['a host inside a sentence', 'Sign in at paypal.com to confirm'],
    ['a URL inside a sentence', 'Confirm here: https://paypal.com/signin now'],
  ])('flags a mismatch hidden behind %s', (_, label) => {
    expect(assessExternalLink('https://evil.example/login', label)).toMatchObject({
      displayedHostname: 'paypal.com',
      requiresConfirmation: true,
      risks: ['display_host_mismatch'],
    });
  });

  it('treats a lookalike in another script as a mismatch, even against the real domain', () => {
    // Cyrillic "а" (U+0430) in place of the Latin "a".
    expect(assessExternalLink('https://evil.example/login', 'p\u0430ypal.com')).toMatchObject({
      displayedHostname: 'p\u0430ypal.com',
      risks: ['display_host_mismatch'],
    });
    expect(assessExternalLink('https://www.paypal.com/', 'p\u0430ypal.com').risks)
      .toContain('display_host_mismatch');
  });

  it('accepts a host named in running text when it is the destination', () => {
    expect(assessExternalLink('https://www.paypal.com/signin', 'Sign in at PayPal.com to confirm'))
      .toMatchObject({ requiresConfirmation: false, risks: [] });
  });

  it('does not read file names, addresses or paths in running text as hosts', () => {
    for (const label of [
      'Download report.pdf',
      'Questions? Write to support@paypal.com',
      'See docs.example.org/paypal.com for details',
    ]) {
      expect(assessExternalLink('https://docs.example.org/download', label).risks, label)
        .not.toContain('display_host_mismatch');
    }
  });

  it('reduces link hosts to their registrable trust domain', () => {
    expect(linkRegistrableDomain('Shop.Example.com')).toBe('example.com');
    expect(linkRegistrableDomain('deep.sub.trusted.io')).toBe('trusted.io');
    // Hosts without a registrable domain stay individually addressable.
    expect(linkRegistrableDomain('127.0.0.1')).toBe('127.0.0.1');
    expect(linkRegistrableDomain('localhost')).toBe('localhost');
    expect(linkRegistrableDomain('')).toBe('');
  });

  it('parses mailto links for composing inside Satchel', () => {
    expect(parseMailtoLink('mailto:mara@example.com?subject=Hallo&body=Guten%20Tag')).toEqual({
      href: 'mailto:mara@example.com?subject=Hallo&body=Guten%20Tag',
      recipients: ['mara@example.com'],
      subject: 'Hallo',
      body: 'Guten Tag',
    });
    expect(parseMailtoLink('mailto:not-an-address')).toBeNull();
  });

  it('keeps umlauts, plus signs and encoded spaces of a mailto link as written', () => {
    const target = parseMailtoLink(
      'mailto:mara@example.com,tom@example.com?subject=Gr%C3%BC%C3%9Fe%20a+b&body=1+1%3D2%0D%0AZeile%202',
    );

    expect(target?.recipients).toEqual(['mara@example.com', 'tom@example.com']);
    expect(target?.subject).toBe('Grüße a+b');
    expect(target?.body).toBe('1+1=2\r\nZeile 2');
  });

  it('flattens line breaks in a mailto subject and caps it at a header line', () => {
    const target = parseMailtoLink(`mailto:mara@example.com?Subject=eins%0D%0Azwei${'x'.repeat(1200)}`);

    expect(target?.subject.startsWith('eins zwei')).toBe(true);
    expect(target?.subject).toHaveLength(998);
  });
});
