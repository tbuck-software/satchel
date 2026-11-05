import { describe, expect, it } from 'vitest';
import { webmailFor } from './webmailLinks';

describe('webmailFor', () => {
  it('links a known provider by the address domain, in any case', () => {
    expect(webmailFor('anna@GMX.de')).toBe('https://www.gmx.net/');
    expect(webmailFor('tom@posteo.net')).toBe('https://posteo.de/');
  });

  it('has no link for a provider it does not know', () => {
    expect(webmailFor('tom@satchel.dev')).toBeNull();
    expect(webmailFor('not an address')).toBeNull();
  });
});
