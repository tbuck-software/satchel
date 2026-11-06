import { useComposerStore } from '@/stores/composer';
import { useUiStore } from '@/stores/ui';

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return Boolean(el?.closest?.('input, textarea, select, [contenteditable="true"]'));
}

/**
 * Whether a page that answers its own keys (the cleanup view, Tasks) must
 * leave this keystroke to the rest of the app.
 *
 * Such pages listen in the capture phase, ahead of the global handler. While
 * a `g` sequence is armed, the next key is its second half: `g d` opens
 * Drafts, so the `d` must not also mark the focused row done (in cleanup:
 * archive a sender's whole Inbox). Dialogs, the composer, text fields and
 * modifier chords are not the page's either.
 */
export function pageKeyBlocked(event: KeyboardEvent): boolean {
  if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) return true;
  if (isEditable(event.target)) return true;
  const ui = useUiStore();
  if (ui.modalOpen || ui.keySequenceHint !== '') return true;
  return useComposerStore().isOpen;
}
