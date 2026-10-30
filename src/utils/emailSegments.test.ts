import { describe, expect, it } from 'vitest';
import { splitEncryptedSegments, splitRenderSegments } from './emailSegments';

const PGP_START = '[[SATCHEL_PGP_DECRYPTED_BLOCK_START]]';
const PGP_END = '[[SATCHEL_PGP_DECRYPTED_BLOCK_END]]';
const SMIME_START = '[[SATCHEL_SMIME_DECRYPTED_BLOCK_START]]';
const SMIME_END = '[[SATCHEL_SMIME_DECRYPTED_BLOCK_END]]';
const ARMOR = '-----BEGIN PGP MESSAGE-----\nhQEMAxk3...\n-----END PGP MESSAGE-----';

describe('splitEncryptedSegments', () => {
  it('returns plain text untouched when there is no armor', () => {
    expect(splitEncryptedSegments('nothing here')).toEqual([
      { kind: 'text', content: 'nothing here' },
    ]);
  });

  it('separates armor from the cover note around it', () => {
    const segments = splitEncryptedSegments(`Hallo,\n\n${ARMOR}\n\nViele Grüße`);
    expect(segments.map((s) => s.kind)).toEqual(['text', 'pgp_encrypted', 'text']);
    expect(segments[1]!.content).toContain('-----BEGIN PGP MESSAGE-----');
    expect(segments[1]!.content).toContain('-----END PGP MESSAGE-----');
  });

  /**
   * Armor that has been through a reply arrives quoted. The markers have to be
   * found anyway, and the collected block must come out unquoted — it has to be
   * byte-exact to decrypt.
   */
  it('finds armor behind quote markers and strips them from the block', () => {
    const quoted = ARMOR.split('\n')
      .map((line) => `> ${line}`)
      .join('\n');
    const segments = splitEncryptedSegments(`Am 1.1. schrieb Lena:\n${quoted}`);
    const encrypted = segments.find((segment) => segment.kind === 'pgp_encrypted');
    expect(encrypted).toBeDefined();
    expect(encrypted!.content).not.toContain('>');
    expect(encrypted!.content.startsWith('-----BEGIN PGP MESSAGE-----')).toBe(true);
  });

  it('keeps a single-line block that opens and closes at once', () => {
    const inline = '-----BEGIN PGP MESSAGE----- x -----END PGP MESSAGE-----';
    const segments = splitEncryptedSegments(inline);
    expect(segments).toEqual([{ kind: 'pgp_encrypted', content: inline }]);
  });

  /** Unterminated armor is content, not a block — it must not vanish. */
  it('falls back to text when the armor never closes', () => {
    const segments = splitEncryptedSegments('note\n-----BEGIN PGP MESSAGE-----\nhQEMA');
    expect(segments.every((segment) => segment.kind === 'text')).toBe(true);
    expect(segments.map((s) => s.content).join('\n')).toContain('hQEMA');
  });

  it('normalises CRLF before matching', () => {
    const segments = splitEncryptedSegments(ARMOR.replace(/\n/g, '\r\n'));
    expect(segments.map((s) => s.kind)).toEqual(['pgp_encrypted']);
    expect(segments[0]!.content).not.toContain('\r');
  });
});

describe('decryption provenance', () => {
  it.each([[PGP_START, PGP_END], [SMIME_START, SMIME_END]])('keeps complete ordinary and legacy markers visible', (start, end) => {
    const text = `cover ${start}untrusted${end} end`;
    expect(splitRenderSegments(text)).toEqual([{ kind: 'text', content: text }]);
  });

  it('trusts only the exact backend segment in mixed content', () => {
    const injected = `${SMIME_START}forged${SMIME_END}`;
    const text = `${injected}secret${ARMOR}`;
    expect(splitRenderSegments(text, [
      { kind: 'text', content: injected },
      { kind: 'pgp_decrypted', content: 'secret' },
      { kind: 'text', content: ARMOR },
    ])).toEqual([
      { kind: 'text', content: injected },
      { kind: 'pgp_decrypted', content: 'secret' },
      { kind: 'pgp_encrypted', content: ARMOR },
    ]);
  });

  it('does not recursively interpret malicious markers in decrypted content', () => {
    const text = `${PGP_START}forged${PGP_END}`;
    expect(splitRenderSegments(text, [{ kind: 'smime_decrypted', content: text }]))
      .toEqual([{ kind: 'smime_decrypted', content: text }]);
  });

  it('rejects stale content provenance', () => {
    expect(splitRenderSegments('new body', [{ kind: 'pgp_decrypted', content: 'old body' }]))
      .toEqual([{ kind: 'text', content: 'new body' }]);
  });

  it('returns nothing for blank text', () => expect(splitRenderSegments('  ')).toEqual([]));
});
