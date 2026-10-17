import { describe, expect, it } from 'vitest';
import {
  hasOnlyValidRecipients,
  hasOpenRecipientToken,
  isValidEmailAddress,
  isValidRecipient,
  recipientAddress,
  splitRecipientList,
} from './recipients';

describe('splitRecipientList', () => {
  it('keeps a display name that contains the separator in one token', () => {
    expect(splitRecipientList('"Morgan, Alex" <a.morgan@example.com>')).toEqual([
      '"Morgan, Alex" <a.morgan@example.com>',
    ]);
  });

  it('splits a plain list on comma, semicolon and newline', () => {
    expect(splitRecipientList('a@x.de, b@y.de')).toEqual(['a@x.de', 'b@y.de']);
    expect(splitRecipientList('a@x.de; b@y.de')).toEqual(['a@x.de', 'b@y.de']);
    expect(splitRecipientList('a@x.de\nb@y.de')).toEqual(['a@x.de', 'b@y.de']);
  });

  it('splits a mixed list without cutting the quoted name', () => {
    expect(splitRecipientList('"A, B" <a@x.de>, c@y.de')).toEqual([
      '"A, B" <a@x.de>',
      'c@y.de',
    ]);
  });

  it('ignores separators inside angle brackets and drops empty tokens', () => {
    expect(splitRecipientList('Team <team,list@x.de>,, b@y.de')).toEqual([
      'Team <team,list@x.de>',
      'b@y.de',
    ]);
  });
});

describe('hasOpenRecipientToken', () => {
  it('reports an unfinished quote or angle bracket', () => {
    expect(hasOpenRecipientToken('"Buck')).toBe(true);
    expect(hasOpenRecipientToken('Alex <t@x.de')).toBe(true);
    expect(hasOpenRecipientToken('"Morgan, Alex" <t@x.de>')).toBe(false);
    expect(hasOpenRecipientToken('a@x.de')).toBe(false);
  });
});

describe('recipient validity', () => {
  it('accepts bare addresses and `Name <addr>` tokens', () => {
    expect(isValidEmailAddress('a@x.de')).toBe(true);
    expect(recipientAddress('"Morgan, Alex" <a.morgan@example.com>')).toBe('a.morgan@example.com');
    expect(isValidRecipient('Anna Meier <a.meier@kunde.de>')).toBe(true);
  });

  it('rejects the fragments a naive split produces', () => {
    expect(isValidRecipient('"Buck')).toBe(false);
    expect(isValidRecipient('')).toBe(false);
    expect(isValidRecipient('   ')).toBe(false);
    expect(isValidRecipient('Alex" <a.morgan@example.com')).toBe(false);
    expect(isValidEmailAddress('a@x')).toBe(false);
    expect(isValidEmailAddress('a@@x.de')).toBe(false);
    expect(isValidEmailAddress('a b@x.de')).toBe(false);
  });

  it('judges a whole list', () => {
    expect(hasOnlyValidRecipients(['a@x.de', 'Anna <b@y.de>'])).toBe(true);
    expect(hasOnlyValidRecipients(['a@x.de', '"Buck'])).toBe(false);
    expect(hasOnlyValidRecipients([])).toBe(true);
  });
});
