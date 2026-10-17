import { describe, expect, it } from 'vitest';
import { splitHighlight, stripHighlight } from './highlight';

describe('splitHighlight', () => {
  it('splits marked ranges into segments', () => {
    expect(splitHighlight('Anbei die \u0001Rechnung\u0002 für März')).toEqual([
      { text: 'Anbei die ', marked: false },
      { text: 'Rechnung', marked: true },
      { text: ' für März', marked: false },
    ]);
  });

  it('handles multiple marks and marks at the edges', () => {
    expect(splitHighlight('\u0001Anna\u0002 an \u0001Anna\u0002')).toEqual([
      { text: 'Anna', marked: true },
      { text: ' an ', marked: false },
      { text: 'Anna', marked: true },
    ]);
  });

  it('passes text without markers through as one plain segment', () => {
    expect(splitHighlight('nur Text')).toEqual([{ text: 'nur Text', marked: false }]);
  });

  it('drops stray markers instead of leaking control characters', () => {
    const segments = splitHighlight('kaputt\u0001er Rest');
    expect(segments.map((segment) => segment.text).join('')).toBe('kaputter Rest');
    expect(segments.some((segment) => segment.text.includes('\u0001'))).toBe(false);
  });

  it('returns no segments for an empty string', () => {
    expect(splitHighlight('')).toEqual([]);
  });
});

describe('stripHighlight', () => {
  it('removes the markers and keeps the text', () => {
    expect(stripHighlight('Anbei die \u0001Rechnung\u0002')).toBe('Anbei die Rechnung');
  });
});
