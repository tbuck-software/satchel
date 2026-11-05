import { Table, TableCell, TableHeader, TableRow, createTable, type TableOptions } from '@tiptap/extension-table';
import { mergeAttributes, type Editor } from '@tiptap/vue-3';

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

/** Editor keymap names next to the bindings the tooltips print. */
export const TABLE_SHORTCUTS = {
  insertOrFocusToolbar: { keymap: 'Mod-Alt-t', binding: 'mod+alt+t' },
  addRowBefore: { keymap: 'Mod-Alt-ArrowUp', binding: 'mod+alt+arrowup' },
  addRowAfter: { keymap: 'Mod-Alt-ArrowDown', binding: 'mod+alt+arrowdown' },
  addColumnBefore: { keymap: 'Mod-Alt-ArrowLeft', binding: 'mod+alt+arrowleft' },
  addColumnAfter: { keymap: 'Mod-Alt-ArrowRight', binding: 'mod+alt+arrowright' },
} as const;

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
    .trim();
}

function displayWidth(text: string): number {
  return [...text].length;
}

/**
 * The table as the plain-text part shows it: a pipe grid, columns padded to
 * their widest cell, a rule under a header row. Readable in a monospaced
 * reader and still legible in a proportional one.
 */
export function tablePlainText(table: TableTreeNode): string {
  const rows: string[][] = [];
  let headerRow = false;
  for (let rowIndex = 0; rowIndex < table.childCount; rowIndex += 1) {
    const row = table.child(rowIndex);
    const cells: string[] = [];
    let allHeaders = row.childCount > 0;
    for (let cellIndex = 0; cellIndex < row.childCount; cellIndex += 1) {
      const cell = row.child(cellIndex);
      if (cell.type.name !== 'tableHeader') allHeaders = false;
      cells.push(cellText(cell));
      // A spanning cell keeps the columns after it in place.
      const span = Number(cell.attrs.colspan) || 1;
      for (let extra = 1; extra < span; extra += 1) cells.push('');
    }
    if (rowIndex === 0) headerRow = allHeaders;
    rows.push(cells);
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
  /** Called for the insert shortcut while the caret is already in a table. */
  onToolbarRequest: (() => void) | null;
}

const MailTable = Table.extend<TableOptions & ComposerTableOptions>({
  addOptions() {
    return {
      ...this.parent!(),
      onToolbarRequest: null,
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
    const run = (command: () => boolean) => () => tableAtSelection(this.editor) !== null && command();
    return {
      ...this.parent?.(),
      [TABLE_SHORTCUTS.insertOrFocusToolbar.keymap]: () => {
        if (tableAtSelection(this.editor)) {
          this.options.onToolbarRequest?.();
          return true;
        }
        return insertMailTable(this.editor);
      },
      [TABLE_SHORTCUTS.addRowBefore.keymap]: run(() => this.editor.commands.addRowBefore()),
      [TABLE_SHORTCUTS.addRowAfter.keymap]: run(() => this.editor.commands.addRowAfter()),
      [TABLE_SHORTCUTS.addColumnBefore.keymap]: run(() => this.editor.commands.addColumnBefore()),
      [TABLE_SHORTCUTS.addColumnAfter.keymap]: run(() => this.editor.commands.addColumnAfter()),
    };
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

const MailTableCell = TableCell.extend({ renderHTML: cellRenderHTML('td') });
const MailTableHeader = TableHeader.extend({ renderHTML: cellRenderHTML('th') });

/** The table nodes for the composer, configured for mail. */
export function composerTableExtensions(options: Partial<ComposerTableOptions> = {}) {
  return [
    MailTable.configure({
      // Column dragging writes pixel widths no recipient's client agrees with.
      resizable: false,
      allowTableNodeSelection: false,
      onToolbarRequest: options.onToolbarRequest ?? null,
    }),
    TableRow,
    MailTableHeader,
    MailTableCell,
  ];
}
