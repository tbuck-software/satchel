import { describe, expect, it } from 'vitest';
import type { Message } from '@/types/mail';
import {
  buildForwardBody,
  bytesToBase64,
  formatTimestamp,
  mergeRecipients,
  normalizeVisibleText,
  normalizedAddresses,
  prefixSubject,
} from './composerText';

function message(overrides: Partial<Message> = {}): Message {
  return {
    id: 'msg-1',
    threadId: 'th-1',
    from: { name: 'Lena Becker', email: 'lena@example.com' },
    to: [{ name: 'Ich', email: 'ich@example.com' }],
    subject: 'Angebot',
    bodyText: 'Hallo,\n\nanbei das Angebot.',
    timestamp: '2026-01-01T10:00:00Z',
    isRead: true,
    attachments: [],
    ...overrides,
  } as Message;
}

describe('prefixSubject', () => {
  it('prefixes a plain subject', () => {
    expect(prefixSubject('Angebot', 'Re')).toBe('Re: Angebot');
    expect(prefixSubject('Angebot', 'Fwd')).toBe('Fwd: Angebot');
  });

  it('does not stack its own prefix', () => {
    expect(prefixSubject('Re: Angebot', 'Re')).toBe('Re: Angebot');
    expect(prefixSubject('RE: Angebot', 'Re')).toBe('RE: Angebot');
    expect(prefixSubject('Fwd: Angebot', 'Fwd')).toBe('Fwd: Angebot');
  });

  it('trims before deciding', () => {
    expect(prefixSubject('   Angebot  ', 'Re')).toBe('Re: Angebot');
  });

  it('names an empty subject in English', () => {
    expect(prefixSubject('', 'Re')).toBe('Reply');
    expect(prefixSubject('   ', 'Fwd')).toBe('Forwarded message');
    expect(prefixSubject('Re: ', 'Re')).toBe('Reply');
  });

  /**
   * The case this exists for. Each client recognises only its own prefix, so a
   * German Outlook and an English client answering each other grow the subject
   * by one prefix per message until the actual text is off the edge of the
   * list.
   */
  it('collapses a chain that grew across clients', () => {
    expect(prefixSubject('AW: Re: AW: Re: Angebot Q3', 'Re')).toBe('AW: Angebot Q3');
    expect(prefixSubject('WG: Fwd: WG: Rechnung', 'Fwd')).toBe('Fwd: Rechnung');
  });

  /**
   * Replying mirrors the correspondent's own spelling, so the subject stays
   * put in their list instead of alternating between two forms.
   */
  it.each([
    ['AW: Angebot', 'AW: Angebot'],
    ['SV: Offert', 'SV: Offert'],
    ['Odp: Oferta', 'Odp: Oferta'],
    ['回复: 报价', '回复: 报价'],
    ['Angebot', 'Re: Angebot'],
  ])('mirrors the reply prefix of %o', (input, expected) => {
    expect(prefixSubject(input, 'Re')).toBe(expected);
  });

  /**
   * A forward goes to someone who was not part of the exchange, so it uses
   * this client's own prefix rather than the correspondent's.
   */
  it('forwards under its own prefix', () => {
    expect(prefixSubject('WG: Rechnung', 'Fwd')).toBe('Fwd: Rechnung');
    expect(prefixSubject('TR: Facture', 'Fwd')).toBe('Fwd: Facture');
  });

  /**
   * The other kind survives, once and canonically: forwarding a reply still
   * has to read as a reply.
   */
  it('keeps a single prefix of the other kind', () => {
    expect(prefixSubject('AW: Re: Angebot', 'Fwd')).toBe('Fwd: Re: Angebot');
    expect(prefixSubject('Fwd: WG: Rechnung', 'Re')).toBe('Re: Fwd: Rechnung');
  });

  it('handles counters and spacing variants', () => {
    expect(prefixSubject('Re[2]: Angebot', 'Re')).toBe('Re: Angebot');
    expect(prefixSubject('Re(3): Angebot', 'Re')).toBe('Re: Angebot');
    expect(prefixSubject('Re[2): Angebot', 'Re')).toBe('Re: Re[2): Angebot');
    expect(prefixSubject('RE : Facture', 'Re')).toBe('RE: Facture');
  });

  /**
   * An unrecognised word before a colon is subject text. Single-letter
   * prefixes are left unrecognised on purpose for the same reason — the cost
   * of a false positive is a mangled subject.
   */
  it.each([
    'Wichtig: bitte lesen',
    'Von: Lena',
    'R: Update',
    'Projekt X: Status',
    'Ref: 2026-0042',
    'Rif: ordine 88',
  ])('leaves %o intact', (subject) => {
    expect(prefixSubject(subject, 'Re')).toBe(`Re: ${subject}`);
  });

  it('is stable when applied repeatedly', () => {
    const once = prefixSubject('Angebot', 'Re');
    expect(prefixSubject(once, 'Re')).toBe(once);
    expect(prefixSubject(prefixSubject(once, 'Re'), 'Re')).toBe(once);
  });

  it('never grows past two prefixes, however long the chain', () => {
    let subject = 'Angebot';
    for (let round = 0; round < 12; round += 1) {
      subject = prefixSubject(round % 2 === 0 ? `AW: ${subject}` : subject, 'Re');
    }
    expect(subject.match(/:/g) ?? []).toHaveLength(1);
    expect(subject.endsWith('Angebot')).toBe(true);
  });
});

describe('formatTimestamp', () => {
  it('formats a valid timestamp as en-US in the current time zone', () => {
    const timestamp = '2026-01-01T10:00:00Z';
    const formatted = formatTimestamp(timestamp);
    expect(formatted).not.toBe(timestamp);
    expect(formatted).toBe(new Date(timestamp).toLocaleString('en-US'));
  });

  /**
   * `Date` reports a bad input with a NaN time rather than by throwing, so a
   * try/catch never fires and "Invalid Date" used to reach the forwarded
   * header block — and from there the recipient.
   */
  it.each(['', 'not a date', '2026-13-45'])('returns %o unchanged when unparseable', (value) => {
    expect(formatTimestamp(value)).toBe(value);
  });
});

describe('buildForwardBody', () => {
  it('quotes the header block above the original text', () => {
    const body = buildForwardBody(message());
    expect(body).toContain('---------- Forwarded message ----------');
    expect(body).toContain('From: Lena Becker <lena@example.com>');
    expect(body).toContain('Subject: Angebot');
    expect(body).toContain('To: ich@example.com');
    expect(body).toContain('anbei das Angebot.');
  });

  it('falls back to the bare address when the sender has no name', () => {
    const body = buildForwardBody(message({ from: { email: 'lena@example.com' } as never }));
    expect(body).toContain('From: lena@example.com');
  });

  it('omits the Cc line when there is no Cc', () => {
    expect(buildForwardBody(message())).not.toContain('Cc:');
  });

  it('lists every Cc recipient when there is one', () => {
    const body = buildForwardBody(
      message({ cc: [{ email: 'a@example.com' }, { email: 'b@example.com' }] as never }),
    );
    expect(body).toContain('Cc: a@example.com, b@example.com');
  });

  it('survives a message without body text', () => {
    expect(() => buildForwardBody(message({ bodyText: undefined as never }))).not.toThrow();
  });

  it('keeps an unparseable date readable instead of writing Invalid Date', () => {
    const body = buildForwardBody(message({ timestamp: 'kaputt' }));
    expect(body).toContain('Date: kaputt');
    expect(body).not.toContain('Invalid Date');
  });
});

describe('normalizeVisibleText', () => {
  /**
   * Text pasted from other clients carries characters that render as nothing
   * but compare as different. Without stripping them the dirty check fires on
   * an untouched draft and the composer asks to save it.
   */
  it('removes zero-width characters', () => {
    expect(normalizeVisibleText('a\u200Bb\u200C\u200Dc\u2060d\uFEFFe')).toBe('abcde');
  });

  it('turns non-breaking spaces into ordinary ones', () => {
    expect(normalizeVisibleText('a\u00A0b')).toBe('a b');
  });

  it('normalises line endings and trims', () => {
    expect(normalizeVisibleText('  a\r\nb\rc  ')).toBe('a\nb\nc');
  });

  it('reports an invisible-only string as empty', () => {
    expect(normalizeVisibleText('\u200B\uFEFF   ')).toBe('');
  });
});

describe('normalizedAddresses', () => {
  it('drops entries that are empty once normalised', () => {
    expect(normalizedAddresses(['a@example.com', '  ', '\u200B'])).toEqual(['a@example.com']);
  });
});

describe('mergeRecipients', () => {
  it('appends only what is new', () => {
    expect(mergeRecipients(['a@example.com'], ['b@example.com'])).toEqual([
      'a@example.com',
      'b@example.com',
    ]);
  });

  it('compares case-insensitively and keeps the existing spelling', () => {
    expect(mergeRecipients(['Lena@Example.com'], ['lena@example.com'])).toEqual([
      'Lena@Example.com',
    ]);
  });

  it('does not duplicate within the incoming list either', () => {
    expect(mergeRecipients([], ['a@example.com', 'A@example.com'])).toEqual(['a@example.com']);
  });

  it('leaves the input arrays alone', () => {
    const existing = ['a@example.com'];
    mergeRecipients(existing, ['b@example.com']);
    expect(existing).toEqual(['a@example.com']);
  });
});

describe('bytesToBase64', () => {
  it('encodes bytes', () => {
    expect(bytesToBase64(new Uint8Array([72, 97, 108, 108, 111]))).toBe('SGFsbG8=');
  });

  it('encodes an empty buffer', () => {
    expect(bytesToBase64(new Uint8Array([]))).toBe('');
  });

  /**
   * The chunking exists because `String.fromCharCode` takes its input as
   * arguments: a whole attachment at once overflows the call stack. This runs
   * well past the 0x8000 chunk boundary.
   */
  it('encodes a buffer larger than the chunk size', () => {
    const bytes = new Uint8Array(0x8000 * 2 + 5).map((_, index) => index % 256);
    expect(() => bytesToBase64(bytes)).not.toThrow();
    expect(bytesToBase64(bytes)).toBe(Buffer.from(bytes).toString('base64'));
  });
});
