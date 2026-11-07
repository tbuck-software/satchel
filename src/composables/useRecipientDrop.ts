import { ref } from 'vue';
import type { AppDrag } from '@/utils/appDrag';
import { appDragKind, finishAppDrag, pendingAppDrag, readAppDrag } from '@/utils/appDragTarget';

export interface RecipientDropOptions {
  /** This composer's id in a chip's drag source. */
  composerId: string;
  /** Where a contact dropped outside the recipient fields goes: To, as in Outlook. */
  addToTo(addresses: string[]): void;
  disabled?: () => boolean;
  /** A recipient drag entered (true) or left (false) the composer. */
  onActiveChange?: (active: boolean) => void;
}

/** The recipient fields take their own drops (`RecipientChips.vue`). */
function overField(event: DragEvent): boolean {
  const target = event.target;
  return target instanceof Element && target.closest('[data-recipient-field]') !== null;
}

/**
 * Contacts and chips dragged onto the composer outside the recipient fields.
 * A contact goes to To. A chip of this composer dropped there stays where it
 * was: it is a move that missed, not a new recipient.
 *
 * The handlers belong on the composer's root in the capture phase, beside the
 * mail drop. Over a recipient field they only keep count and let the field
 * decide; anywhere else they stop the drag, so the editor does not paste the
 * address as text.
 */
export function useRecipientDrop(options: RecipientDropOptions) {
  /** A recipient drag is over the composer. */
  const dragActive = ref(false);
  /** It would land in To: the pointer is over the composer, not over a field. */
  const overBody = ref(false);
  let depth = 0;

  const disabled = () => options.disabled?.() ?? false;

  function ownChip(drag: AppDrag | null): boolean {
    return drag?.kind === 'recipients' && drag.source?.composer === options.composerId;
  }

  function setActive(active: boolean) {
    if (dragActive.value === active) return;
    dragActive.value = active;
    if (!active) overBody.value = false;
    options.onActiveChange?.(active);
  }

  /** Over the body: claims the event and says whether a drop would add to To. */
  function claimBody(event: DragEvent): boolean {
    if (overField(event)) {
      overBody.value = false;
      return false;
    }
    event.preventDefault();
    event.stopPropagation();
    const accepts = !disabled() && !ownChip(pendingAppDrag(event));
    overBody.value = accepts;
    if (event.dataTransfer) event.dataTransfer.dropEffect = accepts ? 'copy' : 'none';
    return accepts;
  }

  function onDragEnter(event: DragEvent) {
    if (appDragKind(event) !== 'recipients') return;
    depth += 1;
    // Refused (sending, a mail merge without recipient fields): nothing
    // lights up or opens, the drag is only kept out of the editor.
    if (!disabled()) setActive(true);
    claimBody(event);
  }

  function onDragOver(event: DragEvent) {
    if (appDragKind(event) !== 'recipients') return;
    claimBody(event);
  }

  function onDragLeave(event: DragEvent) {
    if (appDragKind(event) !== 'recipients') return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) setActive(false);
  }

  function onDrop(event: DragEvent) {
    if (appDragKind(event) !== 'recipients') return;
    depth = 0;
    if (overField(event)) {
      // The field's own handler runs after this one, and whoever reacts to the
      // end of the drag (folding Cc/Bcc away) has to see what it added.
      setTimeout(() => {
        if (depth === 0) setActive(false);
      }, 0);
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const drag = readAppDrag(event);
    finishAppDrag();
    if (drag?.kind === 'recipients' && !ownChip(drag) && !disabled()) options.addToTo(drag.addresses);
    setActive(false);
  }

  return { dragActive, overBody, onDragEnter, onDragOver, onDragLeave, onDrop };
}
