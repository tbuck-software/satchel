/**
 * Who answers `/` in Settings. Its own module so the global keyboard handler
 * does not pull the settings search into the startup bundle.
 */
let focusField: (() => void) | null = null;

/** False when no filter field is on screen. */
export function focusSettingsFilter(): boolean {
  if (!focusField) return false;
  focusField();
  return true;
}

export function registerSettingsFilterField(focus: () => void): () => void {
  focusField = focus;
  return () => {
    if (focusField === focus) focusField = null;
  };
}
