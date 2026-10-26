import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'vue/compiler-sfc';
import { describe, expect, it } from 'vitest';

/*
 * AGENTS.md § Frontend states four rules. One of them — the backend boundary —
 * has had a test since day one and is kept; the other three were only written
 * down, and every one of them is broken in the tree. This scan is the missing
 * half: it freezes today's breakage per file and makes the next one fail.
 *
 * Shape follows frontendBackendBoundary.test.ts on purpose: a named list per
 * rule, a count and a reason per entry, a budget that may only fall, and three
 * tests per rule that guard the list itself — exact match, stale entry, budget.
 * Adding a violation therefore costs two deliberate edits: the entry and the
 * budget. Removing the last one from a file means removing its entry.
 */

const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SOURCE_ROOT = join(REPOSITORY_ROOT, 'src');
const TOKENS_CSS = join(SOURCE_ROOT, 'styles', 'tokens.css');

// Everything a user looks at. `src/styles/` is left out: it is where the
// literals belong.
const GUARDED_ROOTS = [
  join(SOURCE_ROOT, 'components'),
  join(SOURCE_ROOT, 'views'),
  join(SOURCE_ROOT, 'viewer'),
  join(SOURCE_ROOT, 'tray'),
];

// The kit itself. A `<button>` in here is the building block, not a bypass of
// it — but an icon-only one still owes UX-15 its label.
const UI_KIT_ROOT = join(SOURCE_ROOT, 'components', 'ui');

type Rule =
  | 'raw-color'
  | 'palette-class'
  | 'raw-button'
  | 'unlabelled-icon-button'
  | 'scale-literal'
  | 'icon-literal'
  | 'motion-literal'
  | 'focus-suppression'
  | 'contrast';

interface Violation {
  file: string;
  line: number;
  rule: Rule;
  detail: string;
  fix: string;
}

interface KnownViolation {
  file: string;
  count: number;
  reason: string;
}

/** A group shares one reason across the files it lists. */
interface KnownGroup {
  reason: string;
  files: Record<string, number>;
}

function flatten(groups: KnownGroup[]): KnownViolation[] {
  return groups.flatMap((group) => Object.entries(group.files)
    .map(([file, count]) => ({ file, count, reason: group.reason })));
}

// ---------------------------------------------------------------------------
// Rule 1 — no raw colours outside the tokens.
// ---------------------------------------------------------------------------

/*
 * The exceptions STYLEGUIDE.md documents, whole-file: values that leave our
 * stylesheet and land somewhere our custom properties do not reach. These are
 * permanent by design, so they carry no count — the ratchet below would only
 * churn on them.
 */
const RAW_COLOR_EXEMPT_FILES: Record<string, string> = {
  'src/components/mail/RichTextEditor.vue':
    'Colours travel out with the composed mail HTML.',
  'src/components/mail/IsolatedEmailDocument.vue':
    'CSS injected into the mail iframe srcdoc, a document without our tokens.',
  'src/components/mail/ThreadPrintSheet.vue':
    'Paper colours in @media print.',
};

/*
 * Everything else, counted per file. The list may shrink and must never grow.
 */
const KNOWN_RAW_COLORS: KnownViolation[] = [
  {
    file: 'src/components/calendar/CalendarWeekGrid.vue',
    count: 1,
    reason: 'Hand-mixed event shadow; --shadow-sm covers it.',
  },
  {
    file: 'src/components/mail/ComposerHost.vue',
    count: 3,
    reason: 'Floating-composer shadow stack, hand-mixed.',
  },
  {
    file: 'src/components/mail/ExternalLinkDialog.vue',
    count: 1,
    reason: 'Dialog shadow; --shadow-lg covers it.',
  },
  {
    file: 'src/components/mail/LinkDestinationPreview.vue',
    count: 1,
    reason: 'Hover-card shadow; --shadow-md covers it.',
  },
  {
    file: 'src/components/mail/ThreadGroupList.vue',
    count: 1,
    reason: 'Sticky-header shadow; --shadow-sm covers it.',
  },
  {
    file: 'src/components/mail/ThreadListItem.vue',
    count: 3,
    reason: 'White-alpha selection fills plus an arbitrary drop-shadow class.',
  },
  {
    file: 'src/components/onboarding/OnboardingFlow.vue',
    count: 13,
    reason: 'Theme-preview swatches and hand-mixed scrims (UI-04).',
  },
  {
    file: 'src/components/settings/SourceManager.vue',
    count: 1,
    reason: 'Default calendar-source colour; belongs in a shared palette.',
  },
  {
    file: 'src/components/settings/SwipeSettings.vue',
    count: 1,
    reason: 'Preview-card shadow; --shadow-md covers it.',
  },
  {
    file: 'src/views/DraftsView.vue',
    count: 1,
    reason: 'Drop-shadow under the draft pencil.',
  },
  {
    file: 'src/views/InboxView.vue',
    count: 2,
    reason: 'Floating-toolbar shadow stack, hand-mixed.',
  },
];

// Only ever lower this. See KNOWN_RAW_COLORS.
const MAX_KNOWN_RAW_COLORS = 28;

// ---------------------------------------------------------------------------
// Rule 2 — no Tailwind default palette, only the classes bridged in tailwind.css.
// ---------------------------------------------------------------------------

/*
 * The one rule of the four the tree kept almost on its own, and now keeps
 * entirely: the last two `text-white` classes became `text-on-accent` once
 * tailwind.css bridged the token (UI-15).
 */
const KNOWN_PALETTE_CLASSES: KnownViolation[] = [];

// Only ever lower this. See KNOWN_PALETTE_CLASSES.
const MAX_KNOWN_PALETTE_CLASSES = 0;

// ---------------------------------------------------------------------------
// Rule 3 — new UI is assembled from src/components/ui/.
// ---------------------------------------------------------------------------

/* All application controls use the kit. These two isolated surfaces have
 * documented reasons for owning their buttons; do not add routine controls. */
const KNOWN_RAW_BUTTON_GROUPS: KnownGroup[] = [
  {
    reason: 'Full-width call-to-action row; a permanent exception (STYLEGUIDE.md).',
    files: { 'src/components/settings/AccountSetup.vue': 1 },
  },
  {
    reason: 'Menu-bar popover; it ships its own minimal stylesheet (tray.css).',
    files: { 'src/tray/TrayPopover.vue': 2 },
  },
];

const KNOWN_RAW_BUTTONS = flatten(KNOWN_RAW_BUTTON_GROUPS);

// Only ever lower this. See KNOWN_RAW_BUTTON_GROUPS.
const MAX_KNOWN_RAW_BUTTONS = 3;

/*
 * Outside the kit a hand-rolled button is already counted above, so this rule
 * watches the kit itself: a button whose whole content is an icon says nothing
 * to a screen reader. It is empty today — IconButton and MenuItem both name
 * themselves — and it stays empty (UX-15).
 */
const KNOWN_UNLABELLED_ICON_BUTTONS: KnownViolation[] = [];

// Only ever lower this. See KNOWN_UNLABELLED_ICON_BUTTONS.
const MAX_KNOWN_UNLABELLED_ICON_BUTTONS = 0;

// ---------------------------------------------------------------------------
// Rule 4 — radii and type sizes come from the scale.
// ---------------------------------------------------------------------------

/*
 * `border-radius` and `font-size` written as a literal, plus their arbitrary
 * Tailwind equivalents `text-[13px]` and `font-[550]`. Five radius steps and
 * nine type steps exist; the tree also uses 9px, 10px, 14px, 21px and 0.9em.
 * UI-11 pulls these back onto the scale.
 */
const KNOWN_SCALE_LITERAL_GROUPS: KnownGroup[] = [
  {
    reason: 'Settings pane sizing its own chrome; UI-11 puts it on the scale.',
    files: {
      'src/components/settings/AccountSetup.vue': 1,
      'src/components/settings/BackupExport.vue': 1,
      'src/components/settings/BlockedSendersEditor.vue': 1,
      'src/components/settings/MobileSyncSettings.vue': 1,
      'src/components/settings/SmartFoldersEditor.vue': 2,
      'src/components/settings/SwipeSettings.vue': 3,
      'src/components/settings/TemplatesEditor.vue': 1,
    },
  },
  {
    reason: 'Mail surface sizing its own chrome; UI-11 puts it on the scale.',
    files: {
      'src/components/mail/ExternalLinkPrompt.vue': 2,
      'src/components/mail/PgpStatusCard.vue': 1,
      'src/components/mail/PlainTextLinks.vue': 2,
      'src/components/mail/RichTextEditor.vue': 4,
      'src/components/mail/SafeMarkdownBody.vue': 10,
      'src/components/mail/ThreadListItem.vue': 2,
      'src/components/mail/ThreadPrintSheet.vue': 1,
    },
  },
  {
    reason: 'Contacts and topics surfaces predating the scale; UI-11 converts them.',
    files: {
      'src/components/topics/TopicDetail.vue': 2,
      'src/components/topics/TopicPickerOverlay.vue': 1,
    },
  },
  {
    reason: 'Onboarding was built as one screen and sizes its own controls.',
    files: {
      'src/components/onboarding/OnboardingFlow.vue': 10,
    },
  },
  {
    reason: 'Kit component with a hard-coded step; the kit has to lead here.',
    files: {
      'src/components/ui/BottomSheet.vue': 1,
    },
  },
  {
    reason: 'View-level layout sizing its own chrome; UI-11 puts it on the scale.',
    files: {
      'src/views/CalendarView.vue': 1,
      'src/views/InboxView.vue': 1,
      'src/views/NewSendersView.vue': 2,
      'src/views/SearchView.vue': 1,
    },
  },
];

const KNOWN_SCALE_LITERALS = flatten(KNOWN_SCALE_LITERAL_GROUPS);

// Only ever lower this. See KNOWN_SCALE_LITERAL_GROUPS.
const MAX_KNOWN_SCALE_LITERALS = 51;

// ---------------------------------------------------------------------------
// Rule 4b — icons share one line weight, a few grids and three sizes.
// ---------------------------------------------------------------------------

/*
 * UI-12 counted fourteen stroke widths, twelve viewBox grids and eighteen
 * render sizes among 284 hand-written SVGs. New icons go through AppIcon; this
 * holds the old ones to the values UI-12 settled on: no 1.4 stroke, a square
 * 16/20/24 grid, and no render size between the 14/16/20 steps.
 *
 * The entries are drawings, not icons: illustrations, the undo ring, brand
 * marks on their own grids.
 */
const KNOWN_ICON_LITERAL_GROUPS: KnownGroup[] = [
  {
    reason: 'Small disclosure and status glyphs drawn on their own 8–13 grid; redraw on 16 with AppIcon size="sm".',
    files: {
      'src/components/calendar/CalendarSidebar.vue': 1,
      'src/components/layout/FolderBrowserPanel.vue': 1,
      'src/components/layout/MailboxTree.vue': 1,
      'src/components/mail/ComposerSignature.vue': 1,
      'src/components/mail/ComposerThreadContext.vue': 1,
      'src/components/mail/MessageRecipientDetails.vue': 1,
      'src/components/onboarding/OnboardingFlow.vue': 1,
      'src/components/settings/CalendarSettings.vue': 1,
      'src/views/ComposeView.vue': 1,
    },
  },
  {
    reason: 'Sidebar and menu-bar glyphs on an 18 grid; redraw on 16 or 20.',
    files: {
      'src/tray/TrayPopover.vue': 1,
    },
  },
  {
    reason: 'Editor formatting glyphs that are wider than tall (list and quote marks).',
    files: {
      'src/components/mail/RichTextEditor.vue': 5,
    },
  },
  {
    reason: 'Drawings, not icons: empty-state illustrations and the undo countdown ring.',
    files: {
      'src/components/mail/ThreadGroupList.vue': 1,
      'src/components/mail/ThreadView.vue': 1,
      'src/components/ui/UndoFab.vue': 1,
    },
  },
];

const KNOWN_ICON_LITERALS = flatten(KNOWN_ICON_LITERAL_GROUPS);

// Only ever lower this. See KNOWN_ICON_LITERAL_GROUPS.
const MAX_KNOWN_ICON_LITERALS = 18;

// ---------------------------------------------------------------------------
// Rule 5 — transitions take their timing from the motion tokens.
// ---------------------------------------------------------------------------

/*
 * `transition` and `transition-duration` written with a literal time. There
 * were 70 of them across 22 files, none matching the three tokens
 * (100/180/280ms) — so the same hover answered in 110ms here and 160ms there.
 * UI-16 rounded every one onto a token, which is why this list is empty and
 * stays empty.
 *
 * Keyframe animations are deliberately out of scope: a skeleton shimmer and a
 * spinner are loops with their own rhythm, and the three steps describe
 * interface transitions, not loops (STYLEGUIDE.md § Motion).
 */
const KNOWN_MOTION_LITERALS: KnownViolation[] = [];

// Only ever lower this. See KNOWN_MOTION_LITERALS.
const MAX_KNOWN_MOTION_LITERALS = 0;

// ---------------------------------------------------------------------------
// Rule 6 — the focus ring is never switched off without drawing a replacement.
// ---------------------------------------------------------------------------

/*
 * `base.css` gives every button, input, textarea, select and link the same
 * ring. `outline: none`, `outline: 0` and Tailwind's `outline-none` take it
 * away, and this is a keyboard-driven client: the ring is the only place the
 * user can read where they are. UI-09 pulled the six hand-made replacements
 * back onto --border-focus/--focus-outline-offset and --focus-ring; what is
 * left here are the places where the ring is drawn by something else, and
 * every one of them names what draws it.
 */
const KNOWN_FOCUS_SUPPRESSION_GROUPS: KnownGroup[] = [
  {
    reason: 'Borderless input inside a bordered box; the box carries the ring.',
    files: {
      'src/components/mail/RecipientChips.vue': 1,
      'src/components/navigation/OmniSearch.vue': 1,
      'src/components/settings/KeybindingEditor.vue': 1,
      'src/components/topics/TopicPickerOverlay.vue': 1,
    },
  },
  {
    reason: 'Editor surface filling its frame; the frame draws --focus-ring.',
    files: {
      'src/components/mail/ComposerSignature.vue': 1,
      'src/components/mail/RichTextEditor.vue': 1,
      'src/views/ComposeView.vue': 1,
    },
  },
  {
    reason: 'Row or card that draws its own keyboard-focus state instead.',
    files: {
      'src/components/mail/MessageBubble.vue': 1,
      'src/components/mail/ThreadListItem.vue': 1,
    },
  },
  {
    reason: 'Overlay focused programmatically; the scrim is the feedback.',
    files: {
      'src/components/mail/SafeMarkdownBody.vue': 1,
    },
  },
];

const KNOWN_FOCUS_SUPPRESSIONS = flatten(KNOWN_FOCUS_SUPPRESSION_GROUPS);

// Only ever lower this. See KNOWN_FOCUS_SUPPRESSION_GROUPS.
const MAX_KNOWN_FOCUS_SUPPRESSIONS = 10;

// ---------------------------------------------------------------------------
// Rule 7 — text on a self-declared surface stays readable in both themes.
// ---------------------------------------------------------------------------

/*
 * Where one rule sets both its own background and a --text-* colour, the pair
 * is resolved against tokens.css for dark and light and measured: 4.5:1 for
 * every text token (WCAG 2.1 AA). These are the pairs that miss it today.
 */
const KNOWN_CONTRAST_MISSES: KnownViolation[] = [
  {
    file: 'src/components/mail/EmailRenderer.vue',
    count: 1,
    reason: 'Raw PGP segment on a 55% overlay, 2.99:1 light.',
  },
  {
    file: 'src/components/mail/SafeMarkdownBody.vue',
    count: 2,
    reason: 'Inline code and image-error fills, 4.34:1 and 2.95:1 light.',
  },
  {
    file: 'src/components/onboarding/OnboardingFlow.vue',
    count: 1,
    reason: 'Close button: --text-tertiary on an 86% raised fill, 3.67:1 light.',
  },
  {
    file: 'src/components/settings/KeybindingEditor.vue',
    count: 1,
    reason: 'Table head on a 66% raised fill, 2.90:1 light.',
  },
  {
    file: 'src/views/DraftsView.vue',
    count: 1,
    reason: 'Draft avatar: --text-secondary on a 56% overlay fill, 4.49:1 light.',
  },
];

// Only ever lower this. See KNOWN_CONTRAST_MISSES.
const MAX_KNOWN_CONTRAST_MISSES = 6;

// ---------------------------------------------------------------------------
// Source model
// ---------------------------------------------------------------------------

interface Block {
  content: string;
  lineOffset: number;
}

interface ParsedSource {
  path: string;
  template: Block | null;
  styles: Block[];
  scripts: Block[];
}

function productionSources(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) return productionSources(path);
    if (!entry.isFile() || !['.ts', '.tsx', '.vue'].includes(extname(entry.name))) return [];
    if (/\.(?:test|spec)\.[^.]+$/.test(entry.name)) return [];
    return [path];
  });
}

/** Blanks a comment while keeping every newline, so line numbers still hold. */
function blank(text: string): string {
  return text.replace(/[^\n]/g, ' ');
}

function stripHtmlComments(text: string): string {
  return text.replace(/<!--[\s\S]*?-->/g, blank);
}

function stripCssComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, blank);
}

function stripScriptComments(text: string): string {
  return stripCssComments(text).replace(/^[ \t]*\/\/[^\n]*/gm, blank);
}

function parseSource(content: string, path: string): ParsedSource {
  if (extname(path) !== '.vue') {
    return { path, template: null, styles: [], scripts: [{ content, lineOffset: 0 }] };
  }

  const { descriptor, errors } = parse(content, { filename: path });
  if (errors.length > 0) {
    throw new Error(`Could not parse ${relative(REPOSITORY_ROOT, path)}: ${errors.join(', ')}`);
  }

  const toBlock = (block: { content: string; loc: { start: { line: number } } }): Block => ({
    content: block.content,
    lineOffset: block.loc.start.line - 1,
  });

  return {
    path,
    template: descriptor.template ? toBlock(descriptor.template) : null,
    styles: descriptor.styles.map(toBlock),
    scripts: [descriptor.script, descriptor.scriptSetup]
      .filter((block) => block !== null)
      .map(toBlock),
  };
}

function lineOf(block: Block, index: number): number {
  let line = block.lineOffset + 1;
  for (let position = 0; position < index; position += 1) {
    if (block.content[position] === '\n') line += 1;
  }
  return line;
}

function matchesIn(
  block: Block,
  pattern: RegExp,
  build: (match: RegExpMatchArray) => Omit<Violation, 'file' | 'line'> | null,
): Omit<Violation, 'file'>[] {
  const found: Omit<Violation, 'file'>[] = [];
  for (const match of block.content.matchAll(pattern)) {
    const violation = build(match);
    // A match may open on the whitespace that ended the previous one; the line
    // the reader has to look at is where its first real character sits.
    const lead = match[0].length - match[0].trimStart().length;
    if (violation) found.push({ ...violation, line: lineOf(block, (match.index ?? 0) + lead) });
  }
  return found;
}

function isUnder(path: string, root: string): boolean {
  return path === root || path.startsWith(`${root}/`);
}

function repoPath(path: string): string {
  return relative(REPOSITORY_ROOT, path);
}

// ---------------------------------------------------------------------------
// Colour resolution — tokens.css for both themes, WCAG 2.1 contrast.
// ---------------------------------------------------------------------------

type Theme = 'dark' | 'light';
type Rgba = [number, number, number, number];

const tokensCss = readFileSync(TOKENS_CSS, 'utf8');

/*
 * Same reading as src/styles/themeContrast.test.ts, which measures four fixed
 * surfaces. That one stays narrow on purpose; this one sweeps the tree.
 */
function tokensFor(theme: Theme): Map<string, string> {
  // The dark palette lives in :root; the light theme overrides a subset.
  const blocks = [/:root\s*\{([\s\S]*?)\n\}/.exec(tokensCss)?.[1] ?? ''];
  if (theme === 'light') {
    blocks.push(/\[data-theme='light'\]\s*\{([\s\S]*?)\n\}/.exec(tokensCss)?.[1] ?? '');
  }
  const tokens = new Map<string, string>();
  for (const block of blocks) {
    for (const match of block.matchAll(/(--[\w-]+):\s*([^;]+);/g)) {
      tokens.set(String(match[1]), String(match[2]).trim());
    }
  }
  return tokens;
}

const THEMES: Record<Theme, Map<string, string>> = {
  dark: tokensFor('dark'),
  light: tokensFor('light'),
};
const THEME_NAMES: Theme[] = ['dark', 'light'];

function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const character of text) {
    if (character === '(') depth += 1;
    if (character === ')') depth -= 1;
    if (character === ',' && depth === 0) {
      parts.push(current);
      current = '';
    } else current += character;
  }
  parts.push(current);
  return parts;
}

/** Resolves a CSS colour to straight RGBA, or null when it is not static. */
function resolveColor(value: string, theme: Theme, depth = 0): Rgba | null {
  if (depth > 8) return null;
  const text = value.trim();

  const short = /^#([0-9a-f]{3})$/i.exec(text);
  if (short?.[1]) {
    const [r, g, b] = [...short[1]].map((digit) => parseInt(`${digit}${digit}`, 16));
    return [r as number, g as number, b as number, 1];
  }
  const long = /^#([0-9a-f]{6})$/i.exec(text);
  if (long?.[1]) {
    const packed = parseInt(long[1], 16);
    return [(packed >> 16) & 0xff, (packed >> 8) & 0xff, packed & 0xff, 1];
  }
  if (text === 'white') return [255, 255, 255, 1];
  if (text === 'black') return [0, 0, 0, 1];
  if (text === 'transparent') return [0, 0, 0, 0];

  const functional = /^rgba?\(([^)]*)\)$/i.exec(text);
  if (functional?.[1]) {
    const parts = functional[1].split(/[\s,/]+/).filter(Boolean);
    if (parts.length < 3) return null;
    const channel = (part: string) => (part.endsWith('%')
      ? (parseFloat(part) / 100) * 255
      : parseFloat(part));
    const alphaPart = parts[3];
    const alpha = alphaPart === undefined
      ? 1
      : (alphaPart.endsWith('%') ? parseFloat(alphaPart) / 100 : parseFloat(alphaPart));
    const resolved: Rgba = [
      channel(parts[0] as string),
      channel(parts[1] as string),
      channel(parts[2] as string),
      alpha,
    ];
    return resolved.some(Number.isNaN) ? null : resolved;
  }

  const variable = /^var\(\s*(--[\w-]+)\s*(?:,([\s\S]*))?\)$/.exec(text);
  if (variable?.[1]) {
    const declared = THEMES[theme].get(variable[1]);
    if (declared) return resolveColor(declared, theme, depth + 1);
    return variable[2] ? resolveColor(variable[2], theme, depth + 1) : null;
  }

  const mix = /^color-mix\(\s*in\s+srgb\s*,([\s\S]+)\)$/i.exec(text);
  if (mix?.[1]) {
    const [first, second] = splitTopLevel(mix[1]).map((argument) => {
      const weighted = /^([\s\S]+?)\s+([\d.]+)%$/.exec(argument.trim());
      return {
        color: resolveColor(weighted?.[1] ?? argument.trim(), theme, depth + 1),
        weight: weighted?.[2] === undefined ? null : parseFloat(weighted[2]) / 100,
      };
    });
    if (!first?.color || !second?.color) return null;
    // One percentage implies the other; neither means an even mix.
    const firstWeight = first.weight ?? (second.weight === null ? 0.5 : 1 - second.weight);
    const secondWeight = second.weight ?? 1 - firstWeight;
    const total = firstWeight + secondWeight;
    if (total <= 0) return null;
    const blend = (a: number, b: number) => (a * firstWeight + b * secondWeight) / total;
    const top = first.color;
    const bottom = second.color;
    return [
      blend(top[0], bottom[0]),
      blend(top[1], bottom[1]),
      blend(top[2], bottom[2]),
      blend(top[3], bottom[3]),
    ];
  }

  return null;
}

/** Composites a translucent colour onto an opaque one. */
function composite(top: Rgba, bottom: Rgba): Rgba {
  const alpha = top[3];
  return [
    top[0] * alpha + bottom[0] * (1 - alpha),
    top[1] * alpha + bottom[1] * (1 - alpha),
    top[2] * alpha + bottom[2] * (1 - alpha),
    1,
  ];
}

/** WCAG 2.1 relative luminance. */
function luminance([r, g, b]: Rgba): number {
  const channel = (value: number) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrastRatio(a: Rgba, b: Rgba): number {
  const first = luminance(a);
  const second = luminance(b);
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05);
}

// ---------------------------------------------------------------------------
// The scans
// ---------------------------------------------------------------------------

// A raw colour has to be spelled with one of these, so a file without either
// can skip the scan.
const RAW_COLOR_NEEDLES = ['#', 'rgb', 'white', 'black'];
const HEX_COLOR = /(?<![\w&])#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])/g;
// Not \b: Tailwind arbitrary values join words with `_`, as in
// `drop-shadow-[0_1px_2px_rgba(0,0,0,0.7)]`.
const FUNCTIONAL_COLOR = /(?<![a-zA-Z0-9])rgba?\(/g;
// `url(#gradient)` and `href="#top"` are references, not colours.
const REFERENCE = /\burl\(\s*#[\w-]*\s*\)|(?:href|xlink:href|for|id)\s*=\s*"#[^"]*"/g;
/*
 * A declaration whose whole value is `white` or `black`. It reads as a
 * decision that was never made — "white because it sits on blue" and "white
 * because nobody looked" are the same three letters (UI-15). Only the whole
 * value: `color-mix(…, white 8%)` is a mix over a token, not a raw colour.
 */
const KEYWORD_COLOR = /(?<![-\w])[a-z-]+\s*:\s*(?:white|black)\s*(?=[;}])/g;

function rawColorsIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  const template = source.template
    ? [{ ...source.template, content: stripHtmlComments(source.template.content) }]
    : [];
  const blocks = [
    ...template,
    ...source.styles.map((block) => ({ ...block, content: stripCssComments(block.content) })),
    ...source.scripts.map((block) => ({ ...block, content: stripScriptComments(block.content) })),
  ].map((block) => ({ ...block, content: block.content.replace(REFERENCE, blank) }));

  return blocks.flatMap((block) => [
    ...matchesIn(block, HEX_COLOR, (match) => ({
      rule: 'raw-color',
      detail: `hex literal ${match[0]}`,
      fix: 'use a token from src/styles/tokens.css — a literal breaks the light theme',
    })),
    ...matchesIn(block, FUNCTIONAL_COLOR, () => ({
      rule: 'raw-color',
      detail: 'rgb()/rgba() literal',
      fix: 'use a token, or color-mix() over one — a literal breaks the light theme',
    })),
  ]);
}

/** The keyword form, styles only: `fill="white"` on an SVG is not this. */
function keywordColorsIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  return source.styles
    .map((block) => ({ ...block, content: stripCssComments(block.content) }))
    .flatMap((block) => matchesIn(block, KEYWORD_COLOR, (match) => ({
      rule: 'raw-color' as const,
      detail: `keyword colour ${match[0].trim()}`,
      fix: 'use --text-on-accent, --email-canvas or the token the fill belongs to',
    })));
}

const TAILWIND_PALETTE = [
  'slate', 'gray', 'grey', 'zinc', 'neutral', 'stone', 'red', 'orange', 'amber', 'yellow',
  'lime', 'green', 'emerald', 'teal', 'cyan', 'sky', 'blue', 'indigo', 'violet', 'purple',
  'fuchsia', 'pink', 'rose',
].join('|');
const TAILWIND_UTILITY = [
  'text', 'bg', 'border', 'ring', 'fill', 'stroke', 'divide', 'from', 'to', 'via',
  'outline', 'decoration', 'placeholder', 'caret', 'accent', 'shadow',
].join('|');
// The leading guard keeps `var(--accent-primary)` and `--text-primary` out of it.
const PALETTE_CLASS = new RegExp(
  `(?<![-\\w])(?:${TAILWIND_UTILITY})-`
  + `(?:(?:${TAILWIND_PALETTE})-(?:950|[1-9]00|50)(?![0-9])|white|black)`
  + '(?:\\/(?:\\[[^\\]]*\\]|[0-9.]+))?',
  'g',
);

function paletteClassesIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  if (!source.template) return [];
  const block = { ...source.template, content: stripHtmlComments(source.template.content) };
  return matchesIn(block, PALETTE_CLASS, (match) => ({
    rule: 'palette-class',
    detail: `Tailwind default-palette class ${match[0]}`,
    fix: 'use a bridged token class (text-primary, bg-raised, …) from src/styles/tailwind.css',
  }));
}

const BUTTON_ELEMENT = /<button\b([^>]*)>([\s\S]*?)<\/button>/g;
const ICON_MARKUP = /<svg\b[\s\S]*?<\/svg>|<(?:component|[A-Z][\w.]*)\b[^>]*\/>/g;

/** True when the button renders icons and nothing a reader could hear. */
function isIconOnly(inner: string): boolean {
  if (!ICON_MARKUP.test(inner)) {
    ICON_MARKUP.lastIndex = 0;
    return false;
  }
  ICON_MARKUP.lastIndex = 0;
  return inner.replace(ICON_MARKUP, '').trim() === '';
}

function buttonsIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  if (!source.template) return [];
  const block = { ...source.template, content: stripHtmlComments(source.template.content) };
  const inKit = isUnder(source.path, UI_KIT_ROOT);

  return matchesIn(block, BUTTON_ELEMENT, (match) => {
    const attributes = match[1] ?? '';
    const inner = (match[2] ?? '').trim();
    // Outside the kit the element itself is the violation, so it is reported
    // once, as that. Inside the kit a button is legitimate — and then UX-15's
    // question is whether it says its own name.
    if (!inKit) {
      return {
        rule: 'raw-button',
        detail: 'hand-rolled <button>',
        fix: 'compose from src/components/ui/ — AppButton, IconButton, MenuItem or NavItem',
      };
    }
    const labelled = /(?::|v-bind:)?aria-label(?:ledby)?\s*=/.test(attributes)
      // A button hidden from the accessibility tree has nothing to announce.
      || /\baria-hidden\s*=\s*"true"/.test(attributes);
    if (isIconOnly(inner) && !labelled) {
      return {
        rule: 'unlabelled-icon-button',
        detail: 'icon-only <button> without an accessible name',
        fix: 'add :aria-label with a t() string, the way IconButton does (UX-15)',
      };
    }
    return null;
  });
}

// A radius or size that is a shape or an inheritance, not a step on the scale.
const SCALE_EXEMPT_VALUES = new Set(['0', '50%', 'inherit', 'unset', 'initial', 'revert', 'none']);
const SCALE_DECLARATION = /\b(border-radius|font-size)\s*:\s*([^;{}]+)/g;
const ARBITRARY_TEXT_SIZE = /(?<![-\w])text-\[\d+(?:\.\d+)?(?:px|rem|em)\]/g;
const ARBITRARY_FONT = /(?<![-\w])font-\[[^\]]+\]/g;

function scaleLiteralsIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  const styles = source.styles.map((block) => ({
    ...block,
    content: stripCssComments(block.content),
  }));
  const fromStyles = styles.flatMap((block) => matchesIn(block, SCALE_DECLARATION, (match) => {
    const property = match[1] as string;
    const value = (match[2] as string).trim();
    // `${…}` is a runtime value (mail zoom), not a design decision.
    if (value.includes('var(') || value.includes('$')) return null;
    if (SCALE_EXEMPT_VALUES.has(value)) return null;
    const token = property === 'border-radius' ? '--radius-md' : '--font-size-md';
    return {
      rule: 'scale-literal',
      detail: `${property}: ${value}`,
      fix: `use the scale, e.g. var(${token}) — see STYLEGUIDE.md`,
    };
  }));

  if (!source.template) return fromStyles;
  const template = { ...source.template, content: stripHtmlComments(source.template.content) };
  return [
    ...fromStyles,
    ...matchesIn(template, ARBITRARY_TEXT_SIZE, (match) => ({
      rule: 'scale-literal',
      detail: `arbitrary type size ${match[0]}`,
      fix: 'use a token class, e.g. text-[length:var(--font-size-sm)]',
    })),
    ...matchesIn(template, ARBITRARY_FONT, (match) => ({
      rule: 'scale-literal',
      detail: `arbitrary font utility ${match[0]}`,
      fix: 'use a documented weight: font-normal, font-medium, font-semibold, font-bold',
    })),
  ];
}

const SVG_OPEN_TAG = /<svg\b[^>]*>/g;
const ICON_GRIDS = new Set(['0 0 16 16', '0 0 20 20', '0 0 24 24']);
const OFF_STEP_SIZES = new Set(['13', '15', '17', '19']);

function iconLiteralsIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  if (!source.template) return [];
  const template = { ...source.template, content: stripHtmlComments(source.template.content) };
  const tags = matchesIn(template, SVG_OPEN_TAG, (match) => {
    const tag = match[0];
    const problems: string[] = [];
    const viewBox = tag.match(/\bviewBox="([^"]*)"/)?.[1];
    if (viewBox && !ICON_GRIDS.has(viewBox.trim())) problems.push(`viewBox "${viewBox}"`);
    const width = tag.match(/(?<![-\w:])width="(\d+)"/)?.[1];
    if (width && OFF_STEP_SIZES.has(width)) problems.push(`size ${width}px`);
    if (problems.length === 0) return null;
    return {
      rule: 'icon-literal',
      detail: problems.join(', '),
      fix: 'draw it in <AppIcon> (16 grid, size sm|md|lg) — see STYLEGUIDE.md § Icons',
    };
  });
  const strokes = matchesIn(template, /stroke-width="1\.4"/g, () => ({
    rule: 'icon-literal',
    detail: 'stroke-width="1.4"',
    fix: 'icons are stroked at 1.5 — use <AppIcon>',
  }));
  return [...tags, ...strokes];
}

/*
 * A time in a `transition`. The declaration is read whole rather than per
 * value, so `transition: opacity 120ms ease, transform 160ms …` reports twice
 * — once per literal, which is once per decision.
 */
const TRANSITION_DECLARATION = /(?<![-\w])(transition-duration|transition)\s*:\s*([^;{}]*)/g;
const TIME_LITERAL = /(?<![\w.])(?:\d+(?:\.\d+)?|\.\d+)(?:ms|s)(?![\w-])/g;

function motionLiteralsIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  const styles = source.styles.map((block) => ({
    ...block,
    content: stripCssComments(block.content),
  }));

  return styles.flatMap((block) => matchesIn(block, TRANSITION_DECLARATION, (match) => {
    const times = (match[2] ?? '').match(TIME_LITERAL);
    if (!times) return null;
    return {
      rule: 'motion-literal' as const,
      detail: `${match[1]}: literal ${times.join(', ')}`,
      fix: 'use var(--duration-fast|normal|slow) — see STYLEGUIDE.md § Motion',
    };
  }));
}

const FOCUS_SUPPRESSION = /\boutline\s*:\s*(?:none|0)\b/g;
// `focus:outline-none`, `md:focus:outline-none` — the variant prefix ends on a
// colon, which is not part of the class name itself.
const TAILWIND_OUTLINE_NONE = /(?<![-\w])outline-none(?![-\w])/g;

function focusSuppressionsIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  const styles = source.styles.map((block) => ({
    ...block,
    content: stripCssComments(block.content),
  }));
  const fromStyles = styles.flatMap((block) => matchesIn(block, FOCUS_SUPPRESSION, (match) => ({
    rule: 'focus-suppression' as const,
    detail: `${match[0]} switches the focus ring off`,
    fix: 'draw var(--focus-ring), or drop the rule and inherit the ring from base.css',
  })));

  if (!source.template) return fromStyles;
  const template = { ...source.template, content: stripHtmlComments(source.template.content) };
  return [
    ...fromStyles,
    ...matchesIn(template, TAILWIND_OUTLINE_NONE, () => ({
      rule: 'focus-suppression',
      detail: 'outline-none switches the focus ring off',
      fix: 'drop the utility and inherit the ring from base.css',
    })),
  ];
}

/*
 * --text-on-accent, --text-on-amber and --text-inverse are deliberately
 * theme-independent (STYLEGUIDE.md) and sit on a fill the rule rarely declares
 * itself — measuring them against the block's own background says nothing.
 */
const MEASURED_TEXT_TOKENS: Record<string, number> = {
  '--text-primary': 4.5,
  '--text-secondary': 4.5,
  '--text-tertiary': 4.5,
};
const CSS_RULE = /([^{}]*)\{([^{}]*)\}/g;

function contrastMissesIn(source: ParsedSource): Omit<Violation, 'file'>[] {
  const styles = source.styles.map((block) => ({
    ...block,
    content: stripCssComments(block.content),
  }));

  return styles.flatMap((block) => matchesIn(block, CSS_RULE, (match) => {
    const body = match[2] ?? '';
    const background = /(?:^|;)\s*background(?:-color)?\s*:\s*([^;]+)/.exec(body)?.[1]?.trim();
    const foreground = /(?:^|;)\s*color\s*:\s*([^;]+)/.exec(body)?.[1]?.trim();
    if (!background || !foreground) return null;
    const token = /var\(\s*(--text-[\w-]+)/.exec(foreground)?.[1];
    const minimum = token ? MEASURED_TEXT_TOKENS[token] : undefined;
    if (!token || minimum === undefined) return null;

    for (const theme of THEME_NAMES) {
      const base = resolveColor('var(--bg-base)', theme);
      const surface = resolveColor(background, theme);
      const ink = resolveColor(foreground, theme);
      if (!base || !surface || !ink) continue;
      // A translucent surface reads over the page, so compose it there first.
      const opaqueSurface = composite(surface, base);
      const ratio = contrastRatio(composite(ink, opaqueSurface), opaqueSurface);
      if (ratio >= minimum) continue;
      const selector = (match[1] ?? '').trim().replace(/\s+/g, ' ');
      return {
        rule: 'contrast',
        detail: `${selector}: ${token} on ${background} is ${ratio.toFixed(2)}:1 in ${theme}`,
        fix: `needs ${minimum}:1 — pick a darker surface or a stronger text token`,
      };
    }
    return null;
  }));
}

const SCANS: Record<Rule, (source: ParsedSource) => Omit<Violation, 'file'>[]> = {
  'raw-color': (source) => [...rawColorsIn(source), ...keywordColorsIn(source)],
  'palette-class': paletteClassesIn,
  'raw-button': buttonsIn,
  'unlabelled-icon-button': buttonsIn,
  'scale-literal': scaleLiteralsIn,
  'icon-literal': iconLiteralsIn,
  'motion-literal': motionLiteralsIn,
  'focus-suppression': focusSuppressionsIn,
  contrast: contrastMissesIn,
};

function violationsIn(source: ParsedSource, rule: Rule, raw: string): Violation[] {
  if (rule === 'raw-color') {
    if (!RAW_COLOR_NEEDLES.some((needle) => raw.includes(needle))) return [];
    if (repoPath(source.path) in RAW_COLOR_EXEMPT_FILES) return [];
  }
  if (rule === 'raw-button' && isUnder(source.path, UI_KIT_ROOT)) return [];
  return SCANS[rule](source)
    .filter((entry) => entry.rule === rule)
    .map((entry) => ({ ...entry, file: source.path }));
}

function violationsInSource(content: string, path: string, rule: Rule): Violation[] {
  return violationsIn(parseSource(content, path), rule, content);
}

/*
 * One parse per file, shared by all six scans: parsing the 132 guarded SFCs
 * costs ~0.2 s, doing it six times would not. The scans themselves are plain
 * regex passes over the already split blocks, behind a substring gate where
 * one helps.
 */
function allViolations(): Violation[] {
  const rules = Object.keys(SCANS) as Rule[];
  return GUARDED_ROOTS
    .flatMap(productionSources)
    .flatMap((path) => {
      const content = readFileSync(path, 'utf8');
      const source = parseSource(content, path);
      return rules.flatMap((rule) => violationsIn(source, rule, content));
    })
    .map((entry) => ({ ...entry, file: repoPath(entry.file) }));
}

const VIOLATIONS = allViolations();

function countsFor(rule: Rule): Map<string, number> {
  const counts = new Map<string, number>();
  for (const entry of VIOLATIONS) {
    if (entry.rule !== rule) continue;
    counts.set(entry.file, (counts.get(entry.file) ?? 0) + 1);
  }
  return counts;
}

function describeMismatch(rule: Rule, known: Map<string, number>, counts: Map<string, number>) {
  return VIOLATIONS
    .filter((entry) => entry.rule === rule)
    .filter((entry) => (counts.get(entry.file) ?? 0) !== known.get(entry.file))
    .map((entry) => `${entry.file}:${entry.line} ${entry.rule}: ${entry.detail} — ${entry.fix}`)
    .join('\n');
}

/** The three tests that guard a list: exact match, no stale entry, budget. */
function guardTheList(rule: Rule, known: KnownViolation[], budget: number) {
  it('matches the recorded baseline file for file', () => {
    const counts = countsFor(rule);
    const expected = new Map(known.map((entry) => [entry.file, entry.count]));
    const details = describeMismatch(rule, expected, counts);

    expect(Object.fromEntries(counts), details || undefined)
      .toEqual(Object.fromEntries(expected));
  });

  it('carries no stale entry on the known violation list', () => {
    const counts = countsFor(rule);
    const stale = known
      .filter((entry) => (counts.get(entry.file) ?? 0) < entry.count)
      .map((entry) => {
        return `${entry.file} is listed ${entry.count}x, found ${counts.get(entry.file) ?? 0}`;
      });

    expect(stale, 'the list may shrink — pull the number down with the fix').toEqual([]);
  });

  it('holds the known violation list to its budget', () => {
    // A new violation needs an entry *and* a raised budget. Both by hand.
    const total = known.reduce((sum, entry) => sum + entry.count, 0);

    expect(total).toBeLessThanOrEqual(budget);
  });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('design system: no raw colours in components', () => {
  guardTheList('raw-color', KNOWN_RAW_COLORS, MAX_KNOWN_RAW_COLORS);

  it('still scans the files the substring gate has to let through', () => {
    // Guards the gate: every listed violation must survive the prefilter.
    const missed = KNOWN_RAW_COLORS.filter((entry) => {
      const path = join(REPOSITORY_ROOT, entry.file);
      return violationsInSource(readFileSync(path, 'utf8'), path, 'raw-color').length === 0;
    });

    expect(missed).toEqual([]);
  });

  it('keeps the documented exceptions documented', () => {
    // STYLEGUIDE.md names them; a file that stops needing the licence loses it.
    const stillRaw = Object.keys(RAW_COLOR_EXEMPT_FILES).filter((file) => {
      const path = join(REPOSITORY_ROOT, file);
      const parsed = parseSource(readFileSync(path, 'utf8'), path);
      return rawColorsIn(parsed).length + keywordColorsIn(parsed).length > 0;
    });

    expect(stillRaw).toEqual(Object.keys(RAW_COLOR_EXEMPT_FILES));
  });

  it('reports a literal and leaves a token alone', () => {
    const path = join(SOURCE_ROOT, 'components', 'mail', 'Example.vue');
    const source = [
      '<template>',
      '  <i class="drop-shadow-[0_1px_2px_rgba(0,0,0,0.7)]" />',
      '  <svg><path fill="url(#grad)" /></svg>',
      '  <!-- background: #123456 -->',
      '</template>',
      '<style scoped>',
      '.good { background: var(--bg-surface); color: var(--text-primary); }',
      '.mixed { background: color-mix(in srgb, var(--accent-primary) 14%, transparent); }',
      '.bad { background: #1d1f23; }',
      '.also-bad { box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25); }',
      '.keyword { color: white; }',
      '.mix-is-fine { border-color: color-mix(in srgb, var(--border-color) 84%, white 7%); }',
      '/* .commented { color: #fff; } */',
      '</style>',
    ].join('\n');

    const found = violationsInSource(source, path, 'raw-color');

    expect(found.map((entry) => `${entry.line} ${entry.detail}`)).toEqual([
      '2 rgb()/rgba() literal',
      '9 hex literal #1d1f23',
      '10 rgb()/rgba() literal',
      '11 keyword colour color: white',
    ]);
  });
});

describe('design system: no Tailwind default palette', () => {
  guardTheList('palette-class', KNOWN_PALETTE_CLASSES, MAX_KNOWN_PALETTE_CLASSES);

  it('reports palette classes and leaves the bridged token classes alone', () => {
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template>',
      '  <p class="text-primary bg-raised border-border text-accent-primary">ok</p>',
      '  <p class="text-slate-400 bg-red-500/20 text-white bg-white/[0.06]">no</p>',
      '</template>',
      '<style scoped>.x { color: var(--accent-primary); }</style>',
    ].join('\n');

    expect(violationsInSource(source, path, 'palette-class').map((entry) => entry.detail)).toEqual([
      'Tailwind default-palette class text-slate-400',
      'Tailwind default-palette class bg-red-500/20',
      'Tailwind default-palette class text-white',
      'Tailwind default-palette class bg-white/[0.06]',
    ]);
  });
});

describe('design system: UI is assembled from the kit', () => {
  guardTheList('raw-button', KNOWN_RAW_BUTTONS, MAX_KNOWN_RAW_BUTTONS);

  it('reports a hand-rolled button and leaves a kit component alone', () => {
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template>',
      '  <AppButton variant="primary">{{ t(\'send\') }}</AppButton>',
      '  <button type="button" @click="send">{{ t(\'send\') }}</button>',
      '  <!-- <button>ignored</button> -->',
      '</template>',
    ].join('\n');

    expect(violationsInSource(source, path, 'raw-button').map((entry) => entry.line)).toEqual([3]);
  });

  it('leaves the kit itself free to render a button', () => {
    const path = join(UI_KIT_ROOT, 'Example.vue');
    const source = '<template><button type="button"><slot /></button></template>';

    expect(violationsInSource(source, path, 'raw-button')).toEqual([]);
  });
});

describe('design system: icon-only buttons carry a name', () => {
  guardTheList(
    'unlabelled-icon-button',
    KNOWN_UNLABELLED_ICON_BUTTONS,
    MAX_KNOWN_UNLABELLED_ICON_BUTTONS,
  );

  it('reports an unlabelled icon button and accepts a labelled one', () => {
    const path = join(UI_KIT_ROOT, 'Example.vue');
    const source = [
      '<template>',
      '  <button type="button"><svg viewBox="0 0 16 16"><path d="M0 0" /></svg></button>',
      '  <button type="button" :aria-label="title"><svg viewBox="0 0 16 16" /></button>',
      '  <button type="button"><svg viewBox="0 0 16 16" />{{ t(\'label\') }}</button>',
      '</template>',
    ].join('\n');

    expect(violationsInSource(source, path, 'unlabelled-icon-button').map((entry) => entry.line))
      .toEqual([2]);
  });
});

describe('design system: radii and type sizes come from the scale', () => {
  guardTheList('scale-literal', KNOWN_SCALE_LITERALS, MAX_KNOWN_SCALE_LITERALS);

  it('reports literal steps and leaves tokens, shapes and runtime values alone', () => {
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template>',
      '  <p class="text-[13px] font-[550]">no</p>',
      '  <p class="text-[length:var(--font-size-sm)] font-semibold">ok</p>',
      '  <p class="text-[color-mix(in_srgb,var(--text-primary)_78%,var(--text-secondary))]">ok</p>',
      '</template>',
      '<style scoped>',
      '.ok { border-radius: var(--radius-md); font-size: var(--font-size-sm); }',
      '.shape { border-radius: 50%; }',
      '.reset { border-radius: 0; font-size: inherit; }',
      '.bad { border-radius: 9px; font-size: 21px; }',
      '</style>',
    ].join('\n');

    expect(violationsInSource(source, path, 'scale-literal').map((entry) => entry.detail)).toEqual([
      'border-radius: 9px',
      'font-size: 21px',
      'arbitrary type size text-[13px]',
      'arbitrary font utility font-[550]',
    ]);
  });
});

describe('design system: icons share one weight, grid and size scale', () => {
  guardTheList('icon-literal', KNOWN_ICON_LITERALS, MAX_KNOWN_ICON_LITERALS);

  it('reports off-scale icons and leaves AppIcon and scale icons alone', () => {
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template>',
      '  <svg width="16" height="16" viewBox="0 0 16 16" stroke-width="1.5"><path d="M1 1"/></svg>',
      '  <AppIcon size="sm"><path d="M1 1"/></AppIcon>',
      '  <svg width="13" height="13" viewBox="0 0 22 18" stroke-width="1.4"><path d="M1 1"/></svg>',
      '</template>',
    ].join('\n');

    expect(violationsInSource(source, path, 'icon-literal').map((entry) => entry.detail)).toEqual([
      'viewBox "0 0 22 18", size 13px',
      'stroke-width="1.4"',
    ]);
  });
});

describe('design system: transitions take their timing from the tokens', () => {
  guardTheList('motion-literal', KNOWN_MOTION_LITERALS, MAX_KNOWN_MOTION_LITERALS);

  it('reports a literal time and leaves the tokens and the loops alone', () => {
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template><div class="a" /></template>',
      '<style scoped>',
      '.ok { transition: opacity var(--duration-fast) var(--ease-out); }',
      '.loop { animation: pulse 1.2s ease-in-out infinite; }',
      '.bad { transition: opacity 120ms ease, transform .16s ease; }',
      '.also-bad { transition-duration: 150ms; }',
      '</style>',
    ].join('\n');

    expect(violationsInSource(source, path, 'motion-literal').map((entry) => entry.detail)).toEqual([
      'transition: literal 120ms, .16s',
      'transition-duration: literal 150ms',
    ]);
  });
});

describe('design system: the focus ring is not switched off silently', () => {
  guardTheList('focus-suppression', KNOWN_FOCUS_SUPPRESSIONS, MAX_KNOWN_FOCUS_SUPPRESSIONS);

  it('reports every way of switching it off and leaves the replacements alone', () => {
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template>',
      '  <button class="focus:outline-none">no</button>',
      '  <button class="outline-hidden">also no, but not this rule</button>',
      '</template>',
      '<style scoped>',
      '.a:focus-visible { outline: none; }',
      '.b:focus-visible { outline: 0; }',
      '.c:focus-visible { outline: var(--border-focus); outline-offset: 1px; }',
      '.d:focus-visible { box-shadow: var(--focus-ring); }',
      '</style>',
    ].join('\n');

    expect(violationsInSource(source, path, 'focus-suppression').map((entry) => entry.line))
      .toEqual([6, 7, 2]);
  });
});

describe('design system: text on a self-declared surface stays readable', () => {
  guardTheList('contrast', KNOWN_CONTRAST_MISSES, MAX_KNOWN_CONTRAST_MISSES);

  /*
   * Tertiary carries the snippet in every row, timestamps and empty states, so
   * it is text and needs 4.5:1 like the others — on the page, the list, cards,
   * menus and a hovered or selected row.
   */
  const TEXT_SURFACES = [
    '--bg-base', '--bg-list', '--bg-surface', '--bg-raised', '--bg-overlay', '--bg-hover', '--bg-active',
  ];
  it.each(THEME_NAMES)('keeps every text token at 4.5:1 on the shared surfaces in %s', (theme) => {
    const misses = Object.keys(MEASURED_TEXT_TOKENS).flatMap((token) => TEXT_SURFACES.flatMap((surface) => {
      const ratio = contrastRatio(
        resolveColor(`var(${token})`, theme) as Rgba,
        resolveColor(`var(${surface})`, theme) as Rgba,
      );
      return ratio >= 4.5 ? [] : [`${token} on ${surface}: ${ratio.toFixed(2)}:1`];
    }));

    expect(misses).toEqual([]);
  });

  it('matches the numbers from the cross-check', () => {
    const on = (ink: string, surface: string) => contrastRatio(
      resolveColor(ink, 'dark') as Rgba,
      resolveColor(surface, 'dark') as Rgba,
    );

    expect(on('#e2e8f0', '#ffffff')).toBeCloseTo(1.23, 2);
    expect(on('#1c1e22', 'rgb(18, 23, 34)')).toBeCloseTo(1.07, 2);
    expect(on('#5b6069', 'rgb(18, 23, 34)')).toBeCloseTo(2.84, 2);
  });

  it('resolves a token to a different value per theme', () => {
    expect(resolveColor('var(--text-primary)', 'dark')).toEqual([0xe9, 0xed, 0xf3, 1]);
    expect(resolveColor('var(--text-primary)', 'light')).toEqual([0x1c, 0x1b, 0x19, 1]);
  });

  it('reports a thin pair and leaves a readable one alone', () => {
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template><div class="ok" /></template>',
      '<style scoped>',
      '.ok { background: var(--bg-surface); color: var(--text-primary); }',
      '.thin { background: var(--accent-primary); color: var(--text-tertiary); }',
      '</style>',
    ].join('\n');
    const found = violationsInSource(source, path, 'contrast');

    expect(found.map((entry) => `${entry.line} ${entry.detail}`)).toEqual([
      '4 .thin: --text-tertiary on var(--accent-primary) is 1.71:1 in dark',
    ]);
  });

  it('composes a translucent surface over the page before measuring', () => {
    // Both fills are white. Read as opaque white they would behave alike; read
    // over --bg-base only the near-opaque one turns the page pale.
    const path = join(SOURCE_ROOT, 'views', 'Example.vue');
    const source = [
      '<template><div class="pale" /></template>',
      '<style scoped>',
      '.veil { background: rgba(255, 255, 255, 0.04); color: var(--text-primary); }',
      '.pale { background: rgba(255, 255, 255, 0.9); color: var(--text-primary); }',
      '</style>',
    ].join('\n');

    expect(violationsInSource(source, path, 'contrast').map((entry) => entry.detail)).toEqual([
      '.pale: --text-primary on rgba(255, 255, 255, 0.9) is 1.05:1 in dark',
    ]);
  });
});

// ---------------------------------------------------------------------------
// Rule 8 — one focus trap, not one per overlay.
// ---------------------------------------------------------------------------

/*
 * UI-08 built a focus trap into `ModalShell`; UX-14 built `useFocusTrap` for
 * every other overlay. Inside a week the two had drifted: the composable had
 * learned that `[contenteditable]` belongs in the cycle and that a Tab an inner
 * field already answered must be left alone, the shell had not. The drift is
 * the symptom; the second copy is the fault. This rule has no budget — a
 * hand-rolled trap is never the right answer once one exists.
 */
const FOCUS_TRAP_HOME = 'src/composables/useFocusTrap.ts';

interface ScannedSource {
  file: string;
  content: string;
}

/**
 * The two halves that make a focus trap and nothing else: a list of focusable
 * selectors, and a Tab handler that walks it in both directions.
 */
function focusTrapsIn(sources: ScannedSource[]): string[] {
  return sources
    .filter(({ content }) => {
      const listsFocusables = /\[tabindex\]:not\(\[tabindex="-1"\]\)/.test(content);
      return listsFocusables && /shiftKey/.test(content);
    })
    .map(({ file }) => file)
    .sort();
}

describe('design system: there is one focus trap', () => {
  it('reports an overlay that rolls its own', () => {
    const rolled = [
      "const FOCUSABLE = ['button:not([disabled])', '[tabindex]:not([tabindex=\"-1\"])'].join(', ');",
      'function onTab(event: KeyboardEvent) {',
      '  const items = focusables();',
      '  const next = event.shiftKey ? current - 1 : current + 1;',
      '  items[next]?.focus();',
      '}',
    ].join('\n');

    expect(focusTrapsIn([
      { file: FOCUS_TRAP_HOME, content: rolled },
      { file: 'src/components/ui/ModalShell.vue', content: rolled },
    ])).toEqual(['src/components/ui/ModalShell.vue', FOCUS_TRAP_HOME]);
  });

  it('leaves an overlay that asks the composable for one alone', () => {
    const wired = [
      "import { useFocusTrap } from '@/composables/useFocusTrap';",
      "const { onTab } = useFocusTrap(panel, { restingFocus: 'container' });",
      'function onShiftClick(event: MouseEvent) { if (event.shiftKey) extend(); }',
    ].join('\n');

    expect(focusTrapsIn([{ file: 'src/components/ui/ModalShell.vue', content: wired }])).toEqual([]);
  });

  it('finds exactly one in the tree', () => {
    const sources = productionSources(SOURCE_ROOT).map((path) => ({
      file: repoPath(path),
      content: readFileSync(path, 'utf8'),
    }));

    expect(focusTrapsIn(sources)).toEqual([FOCUS_TRAP_HOME]);
  });
});
