import { describe, expect, it } from 'vitest';
import {
  declarationValue,
  fontElementStyle,
  keepTextDeclarations,
  withDeclaration,
} from './inlineTextStyle';

describe('inline text style', () => {
  it('keeps the colour a signature was written in', () => {
    // Verbatim from a real adopted signature, where the colour used to vanish.
    expect(
      keepTextDeclarations('color:#48c725;font-family:Aptos, Calibri, sans-serif;font-size:12pt;'),
    ).toBe('color: #48c725; font-family: Aptos, Calibri, sans-serif; font-size: 12pt');
  });

  it('drops layout so a signature cannot push the composer around', () => {
    expect(
      keepTextDeclarations('color:#333;margin:40px;position:absolute;float:left;width:900px'),
    ).toBe('color: #333');
  });

  it('reports nothing to keep instead of an empty style attribute', () => {
    expect(keepTextDeclarations('margin:0;padding:0')).toBeNull();
    expect(keepTextDeclarations('')).toBeNull();
    expect(keepTextDeclarations(null)).toBeNull();
  });

  it('refuses values that reach outside the mail', () => {
    expect(keepTextDeclarations('background-color:url(https://tracker.example/x.png)')).toBeNull();
    expect(keepTextDeclarations('color:expression(alert(1))')).toBeNull();
    expect(keepTextDeclarations('font-family:"Foo";background-color:#fff')).toBe(
      'font-family: "Foo"; background-color: #fff',
    );
  });

  it('survives malformed declarations rather than losing the rest', () => {
    expect(keepTextDeclarations('color:#111;;font-size;font-weight:600')).toBe(
      'color: #111; font-weight: 600',
    );
  });

  it('changes a colour without dropping the font it sits in', () => {
    expect(withDeclaration('color: #111; font-family: Georgia', 'color', '#d93025')).toBe(
      'font-family: Georgia; color: #d93025',
    );
  });

  it('adds a colour to text that had none', () => {
    expect(withDeclaration(null, 'color', '#1a73e8')).toBe('color: #1a73e8');
    expect(withDeclaration('font-size: 14px', 'color', '#1a73e8')).toBe(
      'font-size: 14px; color: #1a73e8',
    );
  });

  it('reports nothing left when the last declaration is cleared', () => {
    expect(withDeclaration('color: #111', 'color', null)).toBeNull();
    expect(withDeclaration('color: #111; font-size: 14px', 'color', null)).toBe('font-size: 14px');
  });

  it('reads back the colour the caret sits in', () => {
    expect(declarationValue('font-size: 14px; color: #48c725', 'color')).toBe('#48c725');
    expect(declarationValue('font-size: 14px', 'color')).toBeNull();
    expect(declarationValue(null, 'color')).toBeNull();
    // Layout never made it in, so it cannot be read back out either.
    expect(declarationValue('margin: 4px', 'margin')).toBeNull();
  });

  it('translates the legacy font element', () => {
    expect(fontElementStyle('#ff0000', 'Georgia')).toBe('color: #ff0000; font-family: Georgia');
    expect(fontElementStyle('#ff0000', null)).toBe('color: #ff0000');
    expect(fontElementStyle(null, null)).toBeNull();
    expect(fontElementStyle('   ', '')).toBeNull();
  });
});
