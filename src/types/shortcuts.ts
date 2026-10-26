/** What a surface shows for an action's shortcut: a visible label and its `aria-keyshortcuts` form. */
export interface ShortcutHint {
  label: string;
  /** The label as keycaps, one per stroke; `g i` is two. */
  keys: string[];
  aria?: string;
}
