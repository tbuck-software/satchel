import { t } from '@/i18n/runtime';
import {
  DEFAULT_KEYBINDINGS,
  KEYBINDING_SPECS,
  type KeybindingContext,
} from './keybindingDefaults';
import type { ShortcutHint } from '@/types/shortcuts';

export type { ShortcutHint } from '@/types/shortcuts';
export { DEFAULT_KEYBINDINGS, type KeybindingContext } from './keybindingDefaults';

export interface KeybindingDefinition {
  action: string;
  label: string;
  defaultBinding: string;
  context: KeybindingContext;
  keywords?: string;
  aliasOf?: string;
}

/**
 * One registry feeds runtime matching, Settings, command hints and reset.
 * Keeping the visible catalogue next to the defaults prevents a shortcut from
 * working invisibly or appearing in Settings without an implementation.
 *
 * `label` and `keywords` are getters: a module-level table of translated
 * strings would freeze the language at import time.
 */
export const KEYBINDING_DEFINITIONS: readonly KeybindingDefinition[] = KEYBINDING_SPECS.map(
  (spec) => ({
    ...spec,
    get label() {
      return t(`shortcuts.actions.${spec.action}.label`);
    },
    get keywords() {
      return t(`shortcuts.actions.${spec.action}.keywords`);
    },
  }),
);

export const KEYBINDING_CONTEXT_LABELS: Record<KeybindingContext, string> = {
  get global() {
    return t('shortcuts.contexts.global');
  },
  get navigation() {
    return t('shortcuts.contexts.navigation');
  },
  get mailbox() {
    return t('shortcuts.contexts.mailbox');
  },
  get mail() {
    return t('shortcuts.contexts.mail');
  },
};

function normalizedKeyName(key: string): string {
  if (key === ' ') return 'space';
  if (key === 'Escape') return 'escape';
  if (key === 'Enter') return 'enter';
  if (key === 'Backspace') return 'backspace';
  if (key === 'Delete') return 'delete';
  if (key === 'ArrowUp') return 'arrowup';
  if (key === 'ArrowDown') return 'arrowdown';
  if (key.length === 1) return key;
  return key.toLowerCase();
}

/** Canonical stroke used by the editor and matcher, e.g. `mod+shift+k`. */
export function keyboardEventStroke(event: KeyboardEvent): string {
  const parts: string[] = [];
  if (event.metaKey || event.ctrlKey) parts.push('mod');
  if (event.altKey) parts.push('alt');
  if (event.shiftKey && event.key.length !== 1) parts.push('shift');

  let key = normalizedKeyName(event.key);
  if (event.shiftKey && event.key.length === 1 && /[a-z]/i.test(event.key)) {
    key = event.key.toUpperCase();
  }
  parts.push(key);
  return parts.join('+');
}

export function matchesKeybinding(event: KeyboardEvent, binding: string | undefined): boolean {
  if (!binding || binding.includes(' ')) return false;
  return keyboardEventStroke(event) === binding;
}

export function bindingStrokes(binding: string): string[] {
  return binding.trim().split(/\s+/).filter(Boolean);
}

/**
 * Keys wired in `useKeyboard` that cannot be rebound. `action` names the
 * registry action a key also triggers, so labels can fall back to it.
 */
export interface FixedKeybinding {
  binding: string;
  action?: string;
}

export const FIXED_KEYBINDINGS: readonly FixedKeybinding[] = [
  { binding: 'escape', action: 'leaveItem' },
  { binding: 'mod+k', action: 'openCommandPalette' },
  { binding: 'mod+f', action: 'openCommandPalette' },
  { binding: '/', action: 'openCommandPalette' },
  { binding: 'mod+n', action: 'compose' },
  { binding: 'mod+z', action: 'undo' },
  { binding: 'arrowdown', action: 'nextThread' },
  { binding: 'arrowup', action: 'prevThread' },
  { binding: 'shift+arrowdown' },
  { binding: 'shift+arrowup' },
  { binding: 'enter', action: 'openItem' },
  { binding: 'backspace', action: 'trash' },
  { binding: 'delete', action: 'trash' },
];

/** Bare keys that stay on with the single-key switch off: they lead back to everything else. */
const ALWAYS_AVAILABLE_ACTIONS: ReadonlySet<string> = new Set(['openCommandPalette', 'help']);

/** Routes that show a mail list or reader, where `mailbox` and `mail` bindings apply. */
export const MAIL_ROUTES: ReadonlySet<string> = new Set(['inbox', 'thread']);

export interface ShortcutScope {
  route: string | null | undefined;
  /** A dialog, menu, sheet or the palette covers the page. */
  covered: boolean;
}

export function keybindingContextActive(context: KeybindingContext, scope: ShortcutScope): boolean {
  if (scope.covered) return false;
  if (context === 'global' || context === 'navigation') return true;
  return scope.route != null && MAIL_ROUTES.has(scope.route);
}

export interface KeybindingState {
  keybindings: Readonly<Record<string, string>>;
  singleKeyShortcuts: boolean;
}

export function bindingFor(action: string, keybindings: Readonly<Record<string, string>>): string {
  return keybindings[action] ?? DEFAULT_KEYBINDINGS[action] ?? '';
}

/** A binding whose first stroke has no Cmd/Ctrl/Alt, i.e. one typing could trigger. */
export function isSingleKeyBinding(binding: string): boolean {
  const first = bindingStrokes(binding)[0];
  if (!first) return false;
  const parts = first.split('+');
  return !parts.includes('mod') && !parts.includes('alt');
}

/** The single-key switch silences single-key bindings only, never a chord. */
export function keybindingEnabled(action: string, binding: string, singleKeyShortcuts: boolean): boolean {
  if (!binding) return false;
  return singleKeyShortcuts || ALWAYS_AVAILABLE_ACTIONS.has(action) || !isSingleKeyBinding(binding);
}

/** Every binding that currently triggers `action`, the rebindable one first. */
export function effectiveBindings(action: string, state: KeybindingState): string[] {
  const own = bindingFor(action, state.keybindings);
  const result = keybindingEnabled(action, own, state.singleKeyShortcuts) ? [own] : [];
  for (const fixed of FIXED_KEYBINDINGS) {
    if (fixed.action === action) result.push(fixed.binding);
  }
  return result;
}

export function isApplePlatform(
  platform = typeof navigator === 'undefined' ? '' : navigator.platform ?? '',
): boolean {
  return /mac|iphone|ipad|ipod/i.test(platform);
}

const KEY_LABELS: Record<string, { apple: string; other: string }> = {
  space: { apple: 'Space', other: 'Space' },
  escape: { apple: 'Esc', other: 'Esc' },
  enter: { apple: '↵', other: 'Enter' },
  backspace: { apple: '⌫', other: 'Backspace' },
  delete: { apple: '⌦', other: 'Del' },
  tab: { apple: '⇥', other: 'Tab' },
  arrowup: { apple: '↑', other: '↑' },
  arrowdown: { apple: '↓', other: '↓' },
  arrowleft: { apple: '←', other: '←' },
  arrowright: { apple: '→', other: '→' },
};

interface ParsedStroke {
  mod: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

function parseStroke(stroke: string): ParsedStroke {
  const parts = stroke.split('+');
  // A literal `+` arrives as an empty last part.
  const key = parts.at(-1) || '+';
  const shiftedLetter = /^[A-Z]$/.test(key);
  return {
    mod: parts.includes('mod'),
    alt: parts.includes('alt'),
    shift: parts.includes('shift') || shiftedLetter,
    key,
  };
}

/**
 * The visible parts of one stroke, platform-aware: `⇧⌘K` on Apple systems,
 * `Ctrl` `Shift` `K` elsewhere. Bare letters stay lowercase, as in mail clients;
 * after a modifier they read like menu shortcuts.
 */
export function formatBindingStroke(stroke: string, platform?: string): string[] {
  const apple = isApplePlatform(platform);
  const parsed = parseStroke(stroke);
  const named = KEY_LABELS[parsed.key.toLowerCase()];
  const hasModifier = parsed.mod || parsed.alt;
  const key = named
    ? named[apple ? 'apple' : 'other']
    : hasModifier || /^[A-Z]$/.test(parsed.key) ? parsed.key.toUpperCase() : parsed.key;
  const result: string[] = [];
  if (apple) {
    if (parsed.alt) result.push('⌥');
    if (parsed.shift) result.push('⇧');
    if (parsed.mod) result.push('⌘');
  } else {
    if (parsed.mod) result.push('Ctrl');
    if (parsed.alt) result.push('Alt');
    if (parsed.shift) result.push('Shift');
  }
  result.push(key);
  return result;
}

/** One label for a whole binding, e.g. `⇧A`, `Ctrl+K` or `g i`. */
export function formatBinding(binding: string, platform?: string): string {
  const joiner = isApplePlatform(platform) ? '' : '+';
  return bindingStrokes(binding)
    .map((stroke) => formatBindingStroke(stroke, platform).join(joiner))
    .join(' ');
}

const ARIA_KEY_NAMES: Record<string, string> = {
  space: 'Space',
  escape: 'Escape',
  enter: 'Enter',
  backspace: 'Backspace',
  delete: 'Delete',
  tab: 'Tab',
  arrowup: 'ArrowUp',
  arrowdown: 'ArrowDown',
  arrowleft: 'ArrowLeft',
  arrowright: 'ArrowRight',
};

/**
 * The `aria-keyshortcuts` form of a binding. Sequences have none: the
 * attribute separates alternatives with spaces and cannot express `g i`.
 */
export function ariaKeyshortcuts(binding: string, platform?: string): string | undefined {
  const strokes = bindingStrokes(binding);
  if (strokes.length !== 1) return undefined;
  const parsed = parseStroke(strokes[0]!);
  const parts: string[] = [];
  if (parsed.mod) parts.push(isApplePlatform(platform) ? 'Meta' : 'Control');
  if (parsed.alt) parts.push('Alt');
  if (parsed.shift) parts.push('Shift');
  parts.push(ARIA_KEY_NAMES[parsed.key.toLowerCase()] ?? parsed.key.toUpperCase());
  return parts.join('+');
}

/**
 * The hint every surface shows for an action: palette, toolbar, menus and
 * help. Null when nothing triggers the action right now, e.g. a bare key with
 * the single-key switch off.
 */
export function shortcutHint(
  action: string,
  state: KeybindingState,
  platform?: string,
): ShortcutHint | null {
  const binding = effectiveBindings(action, state)[0];
  if (!binding) return null;
  return { label: formatBinding(binding, platform), aria: ariaKeyshortcuts(binding, platform) };
}

export interface KeybindingConflict {
  actions: string[];
  /** Fixed keys that win before this binding is ever looked at. */
  fixed: string[];
}

/**
 * Actions whose binding cannot work as recorded: two actions on one key, a
 * key that shadows another action's sequence, or a key `useKeyboard` handles
 * before any rebindable action.
 */
export function keybindingConflicts(
  keybindings: Readonly<Record<string, string>>,
): Map<string, KeybindingConflict> {
  const result = new Map<string, KeybindingConflict>();
  const entry = (action: string) => {
    let conflict = result.get(action);
    if (!conflict) {
      conflict = { actions: [], fixed: [] };
      result.set(action, conflict);
    }
    return conflict;
  };
  const bound = KEYBINDING_SPECS
    .map((spec) => ({ action: spec.action, strokes: bindingStrokes(bindingFor(spec.action, keybindings)) }))
    .filter((item) => item.strokes.length > 0);

  for (const [leftIndex, left] of bound.entries()) {
    for (const right of bound.slice(leftIndex + 1)) {
      const same = left.strokes.join(' ') === right.strokes.join(' ');
      const shadows = (left.strokes.length === 1) !== (right.strokes.length === 1)
        && left.strokes[0] === right.strokes[0];
      if (same || shadows) {
        entry(left.action).actions.push(right.action);
        entry(right.action).actions.push(left.action);
      }
    }
    const fixed = FIXED_KEYBINDINGS.find((item) => item.binding === left.strokes[0]);
    if (fixed && fixed.action !== left.action) entry(left.action).fixed.push(fixed.binding);
  }
  return result;
}
