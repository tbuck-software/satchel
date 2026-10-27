/**
 * Who answers `/` above a mail list. Its own module, like the Settings
 * filter's, so the global keyboard handler does not pull the list search into
 * its import graph.
 */
let focusField: (() => void) | null = null;

/** False when no list search field is on screen. */
export function focusListSearch(): boolean {
  if (!focusField) return false;
  focusField();
  return true;
}

export function registerListSearchField(focus: () => void): () => void {
  focusField = focus;
  return () => {
    if (focusField === focus) focusField = null;
  };
}
