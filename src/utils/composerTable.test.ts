// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Editor } from '@tiptap/vue-3';
import StarterKit from '@tiptap/starter-kit';
import {
  composerTableExtensions,
  insertMailTable,
  mailTableHtml,
  tableAtSelection,
} from './composerTable';

const editors: Editor[] = [];

function makeEditor(content: string, onToolbarRequest: (() => void) | null = null) {
  const editor = new Editor({
    content,
    extensions: [StarterKit.configure({ link: false }), ...composerTableExtensions({ onToolbarRequest })],
  });
  editors.push(editor);
  return editor;
}

/** What the composer emits: the editor HTML with cell paragraphs flattened. */
function mailHtml(editor: Editor) {
  return mailTableHtml(editor.getHTML());
}

function parse(html: string) {
  const template = document.createElement('template');
  template.innerHTML = html;
  return template.content;
}

/** Type into the cell under the caret. */
function type(editor: Editor, text: string) {
  editor.commands.insertContent(text);
}

function pressKey(editor: Editor, init: KeyboardEventInit) {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  let handled = false;
  editor.view.someProp('handleKeyDown', (handler) => {
    handled = handler(editor.view, event) || handled;
    return handled;
  });
  return handled;
}

afterEach(() => {
  while (editors.length) editors.pop()?.destroy();
});

describe('inserting a table', () => {
  it('replaces an empty line with a 3×3 table whose first row is a header', () => {
    const editor = makeEditor('<p></p>');
    editor.commands.setTextSelection(1);

    expect(insertMailTable(editor)).toBe(true);

    const fragment = parse(mailHtml(editor));
    const rows = fragment.querySelectorAll('table tr');
    expect(rows).toHaveLength(3);
    expect(rows[0]!.querySelectorAll('th')).toHaveLength(3);
    expect(rows[1]!.querySelectorAll('td')).toHaveLength(3);
    // The empty line became the table; a line follows so typing can go on below.
    expect(fragment.firstElementChild?.tagName).toBe('TABLE');
    expect(fragment.lastElementChild?.tagName).toBe('P');
    // The caret waits in the first header cell.
    type(editor, 'Name');
    expect(parse(mailHtml(editor)).querySelector('th')?.textContent).toBe('Name');
  });

  it('goes below the sentence the caret is in instead of splitting it', () => {
    const editor = makeEditor('<p>Before the table</p><p>After</p>');
    editor.commands.setTextSelection(4);

    insertMailTable(editor);

    const children = [...parse(mailHtml(editor)).children].map((element) => element.tagName);
    expect(children).toEqual(['P', 'TABLE', 'P']);
    expect(editor.getText()).toContain('Before the table');
  });

  it('does not nest a table inside a table', () => {
    const editor = makeEditor('<p></p>');
    insertMailTable(editor);

    expect(tableAtSelection(editor)).not.toBeNull();
    expect(insertMailTable(editor)).toBe(false);
    expect(parse(mailHtml(editor)).querySelectorAll('table')).toHaveLength(1);
  });

  it('inserts with Mod-Alt-T, and the same keys inside a table ask for the table toolbar', () => {
    const onToolbarRequest = vi.fn();
    const editor = makeEditor('<p></p>', onToolbarRequest);
    editor.commands.setTextSelection(1);

    expect(pressKey(editor, { key: 't', code: 'KeyT', keyCode: 84, ctrlKey: true, altKey: true })).toBe(true);
    expect(tableAtSelection(editor)).not.toBeNull();
    expect(onToolbarRequest).not.toHaveBeenCalled();

    expect(pressKey(editor, { key: 't', code: 'KeyT', keyCode: 84, ctrlKey: true, altKey: true })).toBe(true);
    expect(onToolbarRequest).toHaveBeenCalledTimes(1);
    expect(parse(mailHtml(editor)).querySelectorAll('table')).toHaveLength(1);
  });
});

describe('editing a table from the keyboard', () => {
  it('moves between cells with Tab and Shift-Tab and grows a row at the end', () => {
    const editor = makeEditor('<p></p>');
    insertMailTable(editor);

    type(editor, 'A');
    pressKey(editor, { key: 'Tab', keyCode: 9 });
    type(editor, 'B');
    pressKey(editor, { key: 'Tab', shiftKey: true, keyCode: 9 });
    type(editor, '1');

    // Arriving in a cell selects what it holds, the way a spreadsheet does.
    const headers = [...parse(mailHtml(editor)).querySelectorAll('th')].map((cell) => cell.textContent);
    expect(headers).toEqual(['1', 'B', '']);

    // Eight Tabs reach the last cell; the ninth adds a row instead of leaving.
    for (let step = 0; step < 9; step += 1) pressKey(editor, { key: 'Tab', keyCode: 9 });
    expect(parse(mailHtml(editor)).querySelectorAll('tr')).toHaveLength(4);
  });

  it('adds rows and columns with Mod-Alt and the arrows', () => {
    const editor = makeEditor('<p></p>');
    insertMailTable(editor);
    const counts = () => {
      const fragment = parse(mailHtml(editor));
      return { rows: fragment.querySelectorAll('tr').length, cols: fragment.querySelector('tr')!.children.length };
    };

    pressKey(editor, { key: 'ArrowDown', keyCode: 40, ctrlKey: true, altKey: true });
    pressKey(editor, { key: 'ArrowUp', keyCode: 38, ctrlKey: true, altKey: true });
    expect(counts()).toEqual({ rows: 5, cols: 3 });

    pressKey(editor, { key: 'ArrowRight', keyCode: 39, ctrlKey: true, altKey: true });
    pressKey(editor, { key: 'ArrowLeft', keyCode: 37, ctrlKey: true, altKey: true });
    expect(counts()).toEqual({ rows: 5, cols: 5 });
  });

  it('removes rows and columns, toggles the header row and deletes the table', () => {
    const editor = makeEditor('<p></p>');
    insertMailTable(editor);

    // The caret sits in the header row, so that is the row that goes.
    editor.commands.deleteRow();
    editor.commands.deleteColumn();
    let fragment = parse(mailHtml(editor));
    expect(fragment.querySelectorAll('tr')).toHaveLength(2);
    expect(fragment.querySelector('tr')!.children).toHaveLength(2);
    expect(fragment.querySelectorAll('th')).toHaveLength(0);

    editor.commands.toggleHeaderRow();
    expect(parse(mailHtml(editor)).querySelectorAll('tr:first-child th')).toHaveLength(2);
    editor.commands.toggleHeaderRow();
    expect(parse(mailHtml(editor)).querySelectorAll('th')).toHaveLength(0);

    editor.commands.deleteTable();
    fragment = parse(mailHtml(editor));
    expect(fragment.querySelector('table')).toBeNull();
  });
});

describe('the HTML that leaves Satchel', () => {
  it('carries borders and padding inline, with attribute fallbacks, and no stylesheet hooks', () => {
    const editor = makeEditor('<p></p>');
    insertMailTable(editor);
    type(editor, 'Item');

    const fragment = parse(mailHtml(editor));
    const table = fragment.querySelector('table')!;
    expect(table.getAttribute('border')).toBe('1');
    expect(table.getAttribute('cellpadding')).toBe('6');
    expect(table.getAttribute('cellspacing')).toBe('0');
    expect(table.getAttribute('style')).toMatch(/border-collapse:\s*collapse/);
    expect(table.getAttribute('style')).toMatch(/border:\s*1px solid/);
    expect(table.querySelector('colgroup')).toBeNull();
    expect(table.hasAttribute('class')).toBe(false);

    for (const cell of table.querySelectorAll('td, th')) {
      const style = cell.getAttribute('style') ?? '';
      expect(style).toMatch(/border:\s*1px solid/);
      expect(style).toMatch(/padding:\s*6px 10px/);
      expect(cell.hasAttribute('class')).toBe(false);
      expect(cell.hasAttribute('colspan')).toBe(false);
      expect(cell.querySelector('p')?.getAttribute('style')).toBe('margin:0');
    }
  });

  it('keeps a cell alignment and spans parsed from incoming HTML', () => {
    const editor = makeEditor(
      '<table><tr><td colspan="2" style="text-align:right">Total</td></tr><tr><td>a</td><td>b</td></tr></table>',
    );
    const cell = parse(mailHtml(editor)).querySelector('td')!;
    expect(cell.getAttribute('colspan')).toBe('2');
    expect(cell.getAttribute('style')).toMatch(/text-align:\s*right/);
    expect(cell.getAttribute('style')).not.toMatch(/text-align:\s*left/);
  });

  it('round-trips through a saved draft unchanged', () => {
    const editor = makeEditor('<p>Numbers:</p>');
    editor.commands.setTextSelection(3);
    insertMailTable(editor);
    type(editor, 'Q1');
    pressKey(editor, { key: 'Tab', keyCode: 9 });
    type(editor, 'Q2');
    pressKey(editor, { key: 'Tab', keyCode: 9 });
    pressKey(editor, { key: 'Tab', keyCode: 9 });
    type(editor, '10');
    const saved = mailHtml(editor);

    const reopened = makeEditor(saved);
    expect(mailHtml(reopened)).toBe(saved);
    expect(reopened.getText()).toBe(editor.getText());
  });

  it('leaves HTML without a table untouched', () => {
    const html = '<p style="color:red">Hi</p>';
    expect(mailTableHtml(html)).toBe(html);
  });
});

describe('the plain-text part', () => {
  it('renders the table as an aligned grid with a rule under the header', () => {
    const editor = makeEditor(
      '<p>Order:</p>'
      + '<table><tr><th>Item</th><th>Qty</th></tr><tr><td>Apples</td><td>12</td></tr><tr><td>Fig</td><td>3</td></tr></table>'
      + '<p>Thanks</p>',
    );

    expect(editor.getText()).toBe([
      'Order:',
      '',
      '| Item   | Qty |',
      '|--------|-----|',
      '| Apples | 12  |',
      '| Fig    | 3   |',
      '',
      'Thanks',
    ].join('\n'));
  });

  it('has no rule without a header row and keeps spanned columns in place', () => {
    const editor = makeEditor(
      '<table><tr><td colspan="2">Wide</td><td>c</td></tr><tr><td>a</td><td>b<br>b2</td><td>c</td></tr></table>',
    );

    expect(editor.getText()).toBe([
      '| Wide |      | c   |',
      '| a    | b b2 | c   |',
    ].join('\n'));
  });
});
