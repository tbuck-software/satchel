import { describe, expect, it } from 'vitest';
import {
  splitDecryptedSegments,
  splitEncryptedSegments,
  splitRenderSegments,
  splitSmimeDecryptedSegments,
} from './emailSegments';

const PGP_START = '[[SATCHEL_PGP_DECRYPTED_BLOCK_START]]';
const PGP_END = '[[SATCHEL_PGP_DECRYPTED_BLOCK_END]]';
const SMIME_START = '[[SATCHEL_SMIME_DECRYPTED_BLOCK_START]]';
const SMIME_END = '[[SATCHEL_SMIME_DECRYPTED_BLOCK_END]]';

const ARMOR = ['-----BEGIN PGP MESSAGE-----', '', 'hQEMAxk3...', '-----END PGP MESSAGE-----'].join(
  '\n',
);

describe('splitDecryptedSegments', () => {
  it('leaves a message without markers as a single text segment', () => {
    expect(splitDecryptedSegments('just a mail')).toEqual([
      { kind: 'text', content: 'just a mail' },
    ]);
  });

  it('lifts a fenced block out from between its surrounding text', () => {
    const segments = splitDecryptedSegments(`before${PGP_START}secret${PGP_END}after`);
    expect(segments).toEqual([
      { kind: 'text', content: 'before' },
      { kind: 'pgp_decrypted', content: 'secret' },
      { kind: 'text', content: 'after' },
    ]);
  });

  it('handles several blocks in one message', () => {
    const input = `${PGP_START}one${PGP_END}mid${PGP_START}two${PGP_END}`;
    expect(splitDecryptedSegments(input).map((s) => s.kind)).toEqual([
      'pgp_decrypted',
      'text',
      'pgp_decrypted',
    ]);
  });

  /**
   * The security-relevant case: a body carrying only an opening marker must not
   * get the decrypted treatment. Unverified content that renders as verified is
   * worse than no decryption at all.
   */
  it('treats an unterminated block as plain text', () => {
    const input = `intro${PGP_START}dangling`;
    const segments = splitDecryptedSegments(input);
    expect(segments.every((segment) => segment.kind === 'text')).toBe(true);
    expect(segments.map((s) => s.content).join('')).toContain('dangling');
  });
});

describe('splitSmimeDecryptedSegments', () => {
  it('lifts a fenced S/MIME block out', () => {
    const segments = splitSmimeDecryptedSegments(`a${SMIME_START}body${SMIME_END}b`);
    expect(segments).toEqual([
      { kind: 'text', content: 'a' },
      { kind: 'smime_decrypted', content: 'body' },
      { kind: 'text', content: 'b' },
    ]);
  });

  /**
   * S/MIME guards only on the opening marker, where PGP requires both. The
   * unterminated case still degrades to text, so the looser guard does not let
   * unverified content through — it only reaches the loop one case earlier.
   */
  it('degrades an unterminated block to text', () => {
    const segments = splitSmimeDecryptedSegments(`intro${SMIME_START}dangling`);
    expect(segments.every((segment) => segment.kind === 'text')).toBe(true);
  });
});

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

describe('splitRenderSegments', () => {
  it('applies all three splitters and drops empty segments', () => {
    const input = `${PGP_START}pgp${PGP_END}\n\n${SMIME_START}smime${SMIME_END}\n\n${ARMOR}`;
    expect(splitRenderSegments(input).map((s) => s.kind)).toEqual([
      'pgp_decrypted',
      'smime_decrypted',
      'pgp_encrypted',
    ]);
  });

  it('finds still-encrypted armor inside the text around a decrypted block', () => {
    const input = `${PGP_START}open${PGP_END}\n${ARMOR}`;
    const kinds = splitRenderSegments(input).map((s) => s.kind);
    expect(kinds).toContain('pgp_decrypted');
    expect(kinds).toContain('pgp_encrypted');
  });

  it('returns nothing for a blank body', () => {
    expect(splitRenderSegments('   \n  ')).toEqual([]);
  });

  it('leaves an ordinary message as one text segment', () => {
    expect(splitRenderSegments('Hallo Welt')).toEqual([{ kind: 'text', content: 'Hallo Welt' }]);
  });
});
