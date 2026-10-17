import { describe, expect, it } from 'vitest';
import { tokenizePlainTextLinks } from './plainTextLinks';

describe('plain text links', () => {
  it('turns HTTP URLs into links while preserving surrounding text', () => {
    expect(tokenizePlainTextLinks('Open https://example.test/a and http://example.test/b.')).toEqual([
      { kind: 'text', text: 'Open ' },
      { kind: 'link', text: 'https://example.test/a', href: 'https://example.test/a' },
      { kind: 'text', text: ' and ' },
      { kind: 'link', text: 'http://example.test/b', href: 'http://example.test/b' },
      { kind: 'text', text: '.' },
    ]);
  });

  it('keeps balanced URL punctuation and excludes sentence punctuation', () => {
    expect(tokenizePlainTextLinks('See (https://example.test/a_(b)).')).toEqual([
      { kind: 'text', text: 'See (' },
      {
        kind: 'link',
        text: 'https://example.test/a_(b)',
        href: 'https://example.test/a_(b)',
      },
      { kind: 'text', text: ').' },
    ]);
  });

  it('does not link non-HTTP schemes', () => {
    expect(tokenizePlainTextLinks('javascript:alert(1) mailto:not-an-address')).toEqual([
      { kind: 'text', text: 'javascript:alert(1) mailto:not-an-address' },
    ]);
  });

  it('turns standalone email addresses into mailto links', () => {
    expect(tokenizePlainTextLinks('Email: mara@example.com.')).toEqual([
      { kind: 'text', text: 'Email: ' },
      { kind: 'link', text: 'mara@example.com', href: 'mailto:mara@example.com' },
      { kind: 'text', text: '.' },
    ]);
  });
});
