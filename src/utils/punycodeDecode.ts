/**
 * IDNA domain → Unicode, decode direction only (RFC 3492).
 *
 * Sender spoofing checks run for the new-sender row at startup, so this sits
 * in the entry chunk; the full `punycode` package (encode, toASCII, UCS-2
 * helpers) cost several KiB there for the one call that turns `xn--` labels
 * back into what a lookalike domain shows. Behaviour matches `toUnicode` from
 * that package (the test compares both); invalid input throws a bare
 * RangeError, which the caller catches — it is never shown.
 */
const BASE = 36;
const T_MIN = 1;
const T_MAX = 26;
const SKEW = 38;
const DAMP = 700;
const MAX_INT = 0x7fffffff;
// The same label separators the package's mapDomain accepts.
const SEPARATORS = /[\x2E。．｡]/g;

function adapt(delta: number, points: number, first: boolean): number {
  let k = 0;
  delta = first ? Math.floor(delta / DAMP) : delta >> 1;
  delta += Math.floor(delta / points);
  for (; delta > ((BASE - T_MIN) * T_MAX) >> 1; k += BASE) {
    delta = Math.floor(delta / (BASE - T_MIN));
  }
  return Math.floor(k + ((BASE - T_MIN + 1) * delta) / (delta + SKEW));
}

function digit(code: number): number {
  if (code >= 0x30 && code < 0x3a) return 26 + (code - 0x30);
  if (code >= 0x41 && code < 0x5b) return code - 0x41;
  if (code >= 0x61 && code < 0x7b) return code - 0x61;
  return BASE;
}

function decodeLabel(input: string): string {
  const output: number[] = [];
  let basic = input.lastIndexOf('-');
  if (basic < 0) basic = 0;
  for (let j = 0; j < basic; j += 1) {
    if (input.charCodeAt(j) >= 0x80) throw new RangeError();
    output.push(input.charCodeAt(j));
  }
  let n = 0x80;
  let bias = 72;
  let i = 0;
  for (let index = basic > 0 ? basic + 1 : 0; index < input.length;) {
    const oldi = i;
    for (let w = 1, k = BASE; ; k += BASE) {
      if (index >= input.length) throw new RangeError();
      const d = digit(input.charCodeAt(index++));
      if (d >= BASE) throw new RangeError();
      if (d > Math.floor((MAX_INT - i) / w)) throw new RangeError();
      i += d * w;
      const t = k <= bias ? T_MIN : k >= bias + T_MAX ? T_MAX : k - bias;
      if (d < t) break;
      if (w > Math.floor(MAX_INT / (BASE - t))) throw new RangeError();
      w *= BASE - t;
    }
    const length = output.length + 1;
    bias = adapt(i - oldi, length, oldi === 0);
    if (Math.floor(i / length) > MAX_INT - n) throw new RangeError();
    n += Math.floor(i / length);
    i %= length;
    output.splice(i++, 0, n);
  }
  return String.fromCodePoint(...output);
}

/** `toUnicode` for a bare domain: only labels starting with `xn--` change. */
export function toUnicode(domain: string): string {
  return domain
    .replace(SEPARATORS, '.')
    .split('.')
    .map((label) => (label.startsWith('xn--') ? decodeLabel(label.slice(4).toLowerCase()) : label))
    .join('.');
}
