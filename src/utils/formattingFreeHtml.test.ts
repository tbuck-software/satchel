// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { formattingFreeText, isFormattingFreeHtml } from './formattingFreeHtml';

describe('isFormattingFreeHtml', () => {
  it('accepts paragraphs and line breaks the composer writes', () => {
    expect(isFormattingFreeHtml('<p>Thanks for the notes.</p><p>Tom<br>Work</p>')).toBe(true);
  });

  it('keeps anything that carries formatting in the frame', () => {
    expect(isFormattingFreeHtml('<p>See <a href="https://example.com">this</a></p>')).toBe(false);
    expect(isFormattingFreeHtml('<p style="color:red">Red</p>')).toBe(false);
    expect(isFormattingFreeHtml('<p><strong>Bold</strong></p>')).toBe(false);
    expect(isFormattingFreeHtml('<table><tr><td>x</td></tr></table>')).toBe(false);
    expect(isFormattingFreeHtml('<p><img src="cid:x"></p>')).toBe(false);
  });

  it('ignores empty bodies', () => {
    expect(isFormattingFreeHtml('')).toBe(false);
    expect(isFormattingFreeHtml('<p> </p>')).toBe(false);
  });
});

describe('formattingFreeText', () => {
  it('keeps paragraphs apart and lines within them together', () => {
    expect(formattingFreeText('<p>Hi Tom,</p><p>A quiet weekend.</p><p>See you!<br>Alex</p>'))
      .toBe('Hi Tom,\n\nA quiet weekend.\n\nSee you!\nAlex');
  });

  it('decodes entities', () => {
    expect(formattingFreeText('<p>Fish &amp; chips &lt;3&#33;</p>')).toBe('Fish & chips <3!');
  });
});
