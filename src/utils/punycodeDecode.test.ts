import { describe, expect, it } from 'vitest';
import { toUnicode as reference } from 'punycode/';
import { toUnicode } from './punycodeDecode';

const SAMPLES = [
  'xn--pple-43d.com', 'xn--80ak6aa92e.com', 'xn--mnchen-3ya.de', 'xn--bcher-kva.example',
  'xn--fiqs8s.xn--fiqz9s', 'xn--n3h.example', 'xn--ls8h.la', 'xn--wgv71a119e.jp',
  'example.com', 'mail.xn--mller-kva.de', 'xn--a.com', 'xn--paypa-uxa.com',
  'a。xn--mnchen-3ya．de', 'xn--ab-fia.example', 'xn--',
];
const INVALID = ['xn--$.com', 'xn--a-ecp.rué', 'xn--zzzzzzzzzzzzzzzzzzzzzzzz.com', 'xn--ü.com'];

describe('toUnicode', () => {
  it.each(SAMPLES)('matches the punycode package for %s', (domain) => {
    expect(toUnicode(domain)).toBe(reference(domain));
  });

  it.each(INVALID)('throws where the package throws, or matches it: %s', (domain) => {
    let expected: string | Error;
    try { expected = reference(domain); } catch (error) { expected = error as Error; }
    if (expected instanceof Error) expect(() => toUnicode(domain)).toThrow(RangeError);
    else expect(toUnicode(domain)).toBe(expected);
  });
});
