import { Table, TableCell, TableHeader, TableRow, createTable, type TableOptions } from '@tiptap/extension-table';
import { mergeAttributes, type Editor } from '@tiptap/vue-3';
import { isApplePlatform } from '@/config/keybindings';

/*
 * Tables in composed mail.
 *
 * Whatever the editor serialises is what leaves Satchel, and recipients read it
 * in clients that drop or ignore stylesheets (Outlook's Word engine, Gmail's
 * class rewriting). Every border and every padding therefore sits on the
 * element itself, inline, with the old presentational attributes as a fallback
 * for clients that strip `style`. The values are mail content, read on the
 * recipient's background, so they are literal and do not follow the app theme.
 */

const MAIL_TABLE_BORDER = '1px solid #c3c7cc';

const MAIL_TABLE_ATTRIBUTES = {
  border: '1',
  cellpadding: '6',
  cellspacing: '0',
  style: `border-collapse:collapse;border:${MAIL_TABLE_BORDER}`,
};

const MAIL_CELL_STYLE = `border:${MAIL_TABLE_BORDER};padding:6px 10px;vertical-align:top;text-align:left`;

/** A new table: three by three, the first row a header. */
export const NEW_TABLE_SIZE = { rows: 3, cols: 3, withHeaderRow: true } as const;

export type TableShortcut =
  | 'insert'
  | 'focusToolbar'
  | 'addRowBefore'
  | 'addRowAfter'
  | 'addColumnBefore'
  | 'addColumnAfter';

/**
 * The table keys, in the app's binding notation (`mod+alt+t`), per platform.
 *
 * macOS keeps ⌘⌥. Elsewhere Ctrl+Alt+T opens a terminal and Ctrl+Alt+arrows
 * switch workspaces on GNOME, KDE, XFCE and Cinnamon, so the table takes
 * Alt+Shift instead. Alt+F10 reaches the toolbar everywhere, as it does in
 * TinyMCE and CKEditor.
 *
 * Not rebindable yet: the shortcut editor has no composer context.
 */
export function tableShortcuts(apple = isApplePlatform()): Record<TableShortcut, string> {
  const chord = apple ? 'mod+alt' : 'alt+shift';
  return {
    insert: `${chord}+t`,
    focusToolbar: 'alt+f10',
    addRowBefore: `${chord}+arrowup`,
    addRowAfter: `${chord}+arrowdown`,
    addColumnBefore: `${chord}+arrowleft`,
    addColumnAfter: `${chord}+arrowright`,
  };
}

const KEYMAP_KEY_NAMES: Record<string, string> = {
  arrowup: 'ArrowUp',
  arrowdown: 'ArrowDown',
  arrowleft: 'ArrowLeft',
  arrowright: 'ArrowRight',
};

/**
 * ProseMirror keymap names for a binding. `mod` is spelled out as the key the
 * labels print for this platform, so the keymap cannot disagree with them. A
 * shifted letter is listed in both cases: browsers report Alt+Shift+T as `T`.
 */
export function keymapNames(binding: string, apple = isApplePlatform()): string[] {
  const parts = binding.split('+');
  const key = parts.pop() ?? '';
  const modifiers = parts.map((part) => ({
    mod: apple ? 'Meta' : 'Ctrl',
    alt: 'Alt',
    shift: 'Shift',
  })[part] ?? part);
  const keyName = KEYMAP_KEY_NAMES[key] ?? (/^f\d+$/.test(key) ? key.toUpperCase() : key);
  const names = [[...modifiers, keyName].join('-')];
  if (parts.includes('shift') && /^[a-z]$/.test(keyName)) {
    names.push([...modifiers, keyName.toUpperCase()].join('-'));
  }
  return names;
}

/**
 * The parts of a ProseMirror node this module reads. Written out because
 * `@tiptap/pm` is not a direct dependency; the real node satisfies it.
 */
interface TableTreeNode {
  type: { name: string };
  attrs: Record<string, unknown>;
  childCount: number;
  child(index: number): TableTreeNode;
  content: { size: number };
  textBetween(
    from: number,
    to: number,
    blockSeparator?: string,
    leafText?: (leaf: TableTreeNode) => string,
  ): string;
}

function cellText(cell: TableTreeNode): string {
  return cell
    .textBetween(0, cell.content.size, ' ', (leaf) => (leaf.type.name === 'hardBreak' ? ' ' : ''))
    .replace(/\s+/g, ' ')
    .trim()
    // A pipe inside a cell would read as a column border.
    .replace(/\|/g, '\\|');
}

// Grapheme clusters are the same in every language (UAX #29): the root locale.
const graphemes = new Intl.Segmenter('und', { granularity: 'grapheme' });

/**
 * East Asian Wide and Fullwidth blocks (Hangul Jamo, CJK, Hangul syllables,
 * compatibility ideographs, fullwidth forms, the supplementary ideograph
 * planes) and emoji shown as pictures: two columns in a monospaced reader.
 */
const WIDE = /^(?:[\u1100-\u115f\u2e80-\u303e\u3041-\ua4cf\uac00-\ud7a3\uf900-\ufaff\ufe30-\ufe4f\uff00-\uff60\uffe0-\uffe6\u{20000}-\u{3fffd}]|\p{Emoji_Presentation})/u;

/** Columns the text takes in a monospaced reader: one per grapheme, two for wide ones. */
export function displayWidth(text: string): number {
  let width = 0;
  for (const { segment } of graphemes.segment(text)) width += WIDE.test(segment) ? 2 : 1;
  return width;
}

/**
 * The table as the plain-text part shows it: a pipe grid, columns padded to
 * their widest cell, a rule under a header row. Readable in a monospaced
 * reader and still legible in a proportional one.
 */
export function tablePlainText(table: TableTreeNode): string {
  const rows: string[][] = [];
  let headerRow = false;
  // Per column: how many rows below still belong to a cell spanning down.
  const carried: number[] = [];
  for (let rowIndex = 0; rowIndex < table.childCount; rowIndex += 1) {
    const row = table.child(rowIndex);
    const cells: string[] = [];
    let column = 0;
    const skipCarried = () => {
      while ((carried[column] ?? 0) > 0) {
        carried[column] = (carried[column] ?? 0) - 1;
        cells[column] = '';
        column += 1;
      }
    };
    let allHeaders = row.childCount > 0;
    for (let cellIndex = 0; cellIndex < row.childCount; cellIndex += 1) {
      const cell = row.child(cellIndex);
      if (cell.type.name !== 'tableHeader') allHeaders = false;
      skipCarried();
      // A spanning cell keeps the columns right of it and the rows under it in place.
      const colspan = Number(cell.attrs.colspan) || 1;
      const rowspan = Number(cell.attrs.rowspan) || 1;
      for (let offset = 0; offset < colspan; offset += 1) {
        cells[column] = offset === 0 ? cellText(cell) : '';
        carried[column] = rowspan - 1;
        column += 1;
      }
    }
    // Spans reaching down past this row's last cell.
    for (; column < carried.length; column += 1) {
      if ((carried[column] ?? 0) > 0) {
        carried[column] = (carried[column] ?? 0) - 1;
        cells[column] = '';
      }
    }
    if (rowIndex === 0) headerRow = allHeaders;
    rows.push(Array.from(cells, (text) => text ?? ''));
  }

  const columns = Math.max(0, ...rows.map((cells) => cells.length));
  if (columns === 0) return '';
  const widths = Array.from({ length: columns }, (_, column) =>
    Math.max(3, ...rows.map((cells) => displayWidth(cells[column] ?? ''))));
  const line = (cells: string[]) =>
    `| ${widths.map((width, column) => {
      const text = cells[column] ?? '';
      return text + ' '.repeat(width - displayWidth(text));
    }).join(' | ')} |`;

  const lines = rows.map(line);
  if (headerRow && lines.length > 0) {
    lines.splice(1, 0, `|${widths.map((width) => '-'.repeat(width + 2)).join('|')}|`);
  }
  return lines.join('\n');
}

/**
 * Paragraphs keep their browser margins in a recipient's client, which turns
 * every cell into a padded box. `renderHTML` cannot see a paragraph's parent,
 * so the cell paragraphs are flattened here, on the serialised HTML.
 */
export function mailTableHtml(html: string): string {
  if (!html.includes('<table')) return html;
  const template = document.createElement('template');
  template.innerHTML = html;
  const paragraphs = template.content.querySelectorAll('td > p, th > p');
  if (paragraphs.length === 0) return html;
  for (const paragraph of paragraphs) {
    const style = paragraph.getAttribute('style');
    paragraph.setAttribute('style', style ? `margin:0;${style}` : 'margin:0');
  }
  return template.innerHTML;
}

/**
 * What the helpers below use of an editor. The Vue editor and the core one an
 * extension sees are different classes; both have these.
 */
type TableEditor = Pick<Editor, 'state' | 'schema' | 'isEditable' | 'chain'>;

/** The table around the caret, as a document position, or null outside one. */
export function tableAtSelection(editor: Pick<TableEditor, 'state'>): { pos: number } | null {
  const { $from } = editor.state.selection;
  for (let depth = $from.depth; depth > 0; depth -= 1) {
    if ($from.node(depth).type.name === 'table') return { pos: $from.before(depth) };
  }
  return null;
}

/**
 * Insert a new table below the block the caret is in, never inside a table and
 * never by splitting the sentence the caret sits in. An empty line becomes the
 * table. A table that ends its container gets a line after it, so typing can
 * continue below without the gap cursor.
 */
export function insertMailTable(editor: TableEditor): boolean {
  if (!editor.isEditable || tableAtSelection(editor)) return false;
  const { $to } = editor.state.selection;
  const table = createTable(
    editor.schema,
    NEW_TABLE_SIZE.rows,
    NEW_TABLE_SIZE.cols,
    NEW_TABLE_SIZE.withHeaderRow,
  ).toJSON();

  let from: number;
  let to: number;
  let endsContainer: boolean;
  if ($to.parent.isTextblock && $to.depth > 0) {
    const blockStart = $to.before();
    const blockEnd = $to.after();
    const empty = $to.parent.content.size === 0;
    from = empty ? blockStart : blockEnd;
    to = blockEnd;
    endsContainer = $to.index($to.depth - 1) === $to.node($to.depth - 1).childCount - 1;
  } else {
    from = $to.pos;
    to = $to.pos;
    endsContainer = $to.index() === $to.parent.childCount;
  }

  const content = endsContainer ? [table, { type: 'paragraph' }] : [table];
  // table > row > header cell > paragraph: the first cell's text starts four in.
  return editor
    .chain()
    .focus()
    .insertContentAt({ from, to }, content, { updateSelection: false })
    .setTextSelection(from + 4)
    .scrollIntoView()
    .run();
}

export interface ComposerTableOptions {
  /** Called for Alt+F10, and for the insert shortcut while the caret is already in a table. */
  onToolbarRequest: (() => void) | null;
  /** Which keys the table uses; see `tableShortcuts`. */
  apple: boolean;
}

const MailTable = Table.extend<TableOptions & ComposerTableOptions>({
  addOptions() {
    return {
      ...this.parent!(),
      onToolbarRequest: null,
      apple: isApplePlatform(),
    };
  },

  renderHTML({ HTMLAttributes }) {
    // No colgroup: its min-widths only steer the editor's own layout.
    return ['table', mergeAttributes(MAIL_TABLE_ATTRIBUTES, HTMLAttributes), ['tbody', 0]];
  },

  renderText({ node }) {
    return tablePlainText(node);
  },

  addKeyboardShortcuts() {
    const { apple } = this.options;
    const keys = tableShortcuts(apple);
    const inTable = () => tableAtSelection(this.editor) !== null;
    const requestToolbar = () => {
      if (!inTable()) return false;
      this.options.onToolbarRequest?.();
      return true;
    };
    const handlers: Record<TableShortcut, () => boolean> = {
      insert: () => requestToolbar() || insertMailTable(this.editor),
      focusToolbar: requestToolbar,
      addRowBefore: () => inTable() && this.editor.commands.addRowBefore(),
      addRowAfter: () => inTable() && this.editor.commands.addRowAfter(),
      addColumnBefore: () => inTable() && this.editor.commands.addColumnBefore(),
      addColumnAfter: () => inTable() && this.editor.commands.addColumnAfter(),
    };
    const shortcuts = { ...this.parent?.() };
    for (const [action, handler] of Object.entries(handlers) as Array<[TableShortcut, () => boolean]>) {
      for (const name of keymapNames(keys[action], apple)) shortcuts[name] = handler;
    }
    return shortcuts;
  },
});

function cellRenderHTML(tag: 'td' | 'th') {
  return ({ HTMLAttributes }: { HTMLAttributes: Record<string, unknown> }) => {
    // Defaults first: an alignment parsed from the cell overrides `text-align`.
    const attributes = mergeAttributes({ style: MAIL_CELL_STYLE }, HTMLAttributes);
    if (attributes.colspan === 1 || attributes.colspan === '1') delete attributes.colspan;
    if (attributes.rowspan === 1 || attributes.rowspan === '1') delete attributes.rowspan;
    return [tag, attributes, 0] as [string, Record<string, unknown>, 0];
  };
}

/**
 * `colwidth` stays in the document for the editor's own column layout, but
 * never reaches the mail: a pasted pixel width means nothing at the other end.
 */
function withoutColwidthInHtml(this: { parent?: () => Record<string, object> }) {
  const attributes = this.parent?.() ?? {};
  return { ...attributes, colwidth: { ...attributes.colwidth, renderHTML: () => ({}) } };
}

const MailTableCell = TableCell.extend({ addAttributes: withoutColwidthInHtml, renderHTML: cellRenderHTML('td') });
const MailTableHeader = TableHeader.extend({ addAttributes: withoutColwidthInHtml, renderHTML: cellRenderHTML('th') });

/** The table nodes for the composer, configured for mail. */
export function composerTableExtensions(options: Partial<ComposerTableOptions> = {}) {
  return [
    MailTable.configure({
      // Column dragging writes pixel widths no recipient's client agrees with.
      resizable: false,
      allowTableNodeSelection: false,
      onToolbarRequest: options.onToolbarRequest ?? null,
      apple: options.apple ?? isApplePlatform(),
    }),
    TableRow,
    MailTableHeader,
    MailTableCell,
  ];
}
