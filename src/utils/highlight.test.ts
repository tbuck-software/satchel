import { describe, expect, it } from 'vitest';
import { highlightMatches, searchHighlightTerms, splitHighlight, stripHighlight } from './highlight';

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

describe('search term highlighting', () => {
  it('uses positive text and subject terms, ignoring filter values and exclusions', () => {
    expect(searchHighlightTerms('from:alice@example.com subject:"Project plan" invoice* -newsletter has:attachment'))
      .toEqual(['Project plan', 'invoice']);
  });

  it('matches literal metacharacters, case and overlapping terms without losing text', () => {
    const text = 'Plan C++ [budget] & planning';
    const segments = highlightMatches(text, ['plan', 'planning', 'C++', '[budget]']);
    expect(segments.filter((part) => part.marked).map((part) => part.text))
      .toEqual(['Plan', 'C++', '[budget]', 'planning']);
    expect(segments.map((part) => part.text).join('')).toBe(text);
    expect(highlightMatches(text, [])).toEqual([{ text, marked: false }]);
  });
});
