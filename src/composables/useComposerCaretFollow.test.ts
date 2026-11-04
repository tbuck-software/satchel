// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { CARET_MARGIN_PX, caretScrollDelta, revealCaret } from './useComposerCaretFollow';

function rect(top: number, bottom: number): DOMRect {
  return { top, bottom, left: 0, right: 300, width: 300, height: bottom - top, x: 0, y: top, toJSON: () => ({}) };
}

describe('caretScrollDelta', () => {
  const view = { top: 50, bottom: 450 };

  it('leaves a caret that is already in view alone', () => {
    expect(caretScrollDelta(view, { top: 200, bottom: 220 })).toBe(0);
  });

  it('scrolls a caret under the docked toolbar up with a margin', () => {
    expect(caretScrollDelta(view, { top: 440, bottom: 460 })).toBe(460 - (450 - CARET_MARGIN_PX));
  });

  it('scrolls back down to a caret above the top edge', () => {
    expect(caretScrollDelta(view, { top: 30, bottom: 50 })).toBe(30 - (50 + CARET_MARGIN_PX));
  });

  it('keeps the line itself visible in a scroller shorter than the margins', () => {
    // 30px of room for a 20px line: 5px each side, not the full margin.
    expect(caretScrollDelta({ top: 0, bottom: 30 }, { top: 40, bottom: 60 })).toBe(60 - 25);
  });
});

describe('revealCaret', () => {
  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('scrolls the column until the focused editor line clears its bottom edge', () => {
    const scroller = document.createElement('div');
    const editor = document.createElement('div');
    editor.contentEditable = 'true';
    const line = document.createElement('p');
    line.textContent = 'The line being typed';
    editor.append(line);
    scroller.append(editor);
    document.body.append(scroller);
    // jsdom has no layout, no scrolling and no isContentEditable.
    editor.tabIndex = 0;
    Object.defineProperty(editor, 'isContentEditable', { value: true });
    let scrolled = 0;
    Object.defineProperty(scroller, 'scrollTop', { get: () => scrolled, set: (value: number) => { scrolled = value; } });
    scroller.getBoundingClientRect = () => rect(52, 460);
    line.getBoundingClientRect = () => rect(470, 494);
    editor.focus();
    const selection = document.getSelection()!;
    selection.selectAllChildren(line);
    selection.collapseToEnd();

    revealCaret(scroller);

    expect(scroller.scrollTop).toBe(494 - (460 - CARET_MARGIN_PX));
  });

  it('does nothing while focus is outside the column', () => {
    const scroller = document.createElement('div');
    const outside = document.createElement('textarea');
    document.body.append(scroller, outside);
    scroller.getBoundingClientRect = () => rect(0, 100);
    outside.focus();

    revealCaret(scroller);

    expect(scroller.scrollTop).toBe(0);
  });
});
