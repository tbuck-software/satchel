import { onBeforeUnmount, watch, type Ref } from 'vue';
import { keyboardVisible } from '@/mobile/keyboardInset';

/**
 * Keeps the line being typed in view while the phone composer scrolls as one
 * column.
 *
 * Fields and body share one scroller there, docked between the navigation bar
 * and the send toolbar, which rides on the keyboard. A body that grows by a
 * line pushes the caret towards the toolbar; nothing in the browser scrolls a
 * caret out from under a docked bar. After each input — and after the keyboard
 * comes up and shrinks the scroller — the caret's line is measured and the
 * scroller moves just far enough to show it with a margin.
 */

/** Room left between the caret's line and the scroller's edges. */
export const CARET_MARGIN_PX = 24;

export interface CaretBox {
  top: number;
  bottom: number;
}

/**
 * How far `scroller` has to scroll so that `caret` sits inside its visible box
 * with `margin` to spare; 0 when it already does.
 */
export function caretScrollDelta(view: CaretBox, caret: CaretBox, margin = CARET_MARGIN_PX): number {
  const room = view.bottom - view.top;
  // A scroller too short for the margin keeps the caret's line at least visible.
  const inset = Math.max(0, Math.min(margin, (room - (caret.bottom - caret.top)) / 2));
  if (caret.bottom > view.bottom - inset) return caret.bottom - (view.bottom - inset);
  if (caret.top < view.top + inset) return caret.top - (view.top + inset);
  return 0;
}

const MIRRORED_STYLES = [
  'box-sizing', 'width', 'padding-top', 'padding-right', 'padding-bottom', 'padding-left',
  'border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width',
  'font-family', 'font-size', 'font-weight', 'font-style', 'letter-spacing', 'line-height',
  'text-transform', 'word-spacing', 'tab-size', 'text-indent',
];

/**
 * The caret's line inside a textarea, in viewport coordinates. A textarea has
 * no selection rects, so its text up to the caret is laid out once more in a
 * hidden copy with the same box and font.
 */
function textareaCaret(textarea: HTMLTextAreaElement): CaretBox {
  const doc = textarea.ownerDocument;
  const style = doc.defaultView?.getComputedStyle(textarea);
  const mirror = doc.createElement('div');
  if (style) {
    for (const name of MIRRORED_STYLES) mirror.style.setProperty(name, style.getPropertyValue(name));
  }
  Object.assign(mirror.style, {
    position: 'absolute',
    top: '0',
    left: '-9999px',
    visibility: 'hidden',
    whiteSpace: 'pre-wrap',
    overflowWrap: 'break-word',
  });
  mirror.textContent = textarea.value.slice(0, textarea.selectionEnd ?? textarea.value.length);
  const marker = doc.createElement('span');
  marker.textContent = '​';
  mirror.append(marker);
  doc.body.append(mirror);
  const offset = marker.offsetTop;
  const lineHeight = marker.offsetHeight || Number.parseFloat(style?.lineHeight ?? '') || 20;
  mirror.remove();
  const rect = textarea.getBoundingClientRect();
  const top = rect.top + offset - textarea.scrollTop;
  return { top, bottom: top + lineHeight };
}

/** The caret's line inside an editable element, in viewport coordinates. */
function selectionCaret(doc: Document): CaretBox | null {
  const selection = doc.getSelection();
  if (!selection?.rangeCount) return null;
  const range = selection.getRangeAt(0).cloneRange();
  range.collapse(false);
  const rect = range.getClientRects?.()[0];
  if (rect && (rect.top !== 0 || rect.bottom !== 0)) return { top: rect.top, bottom: rect.bottom };
  // A caret on an empty line has no rects; its block stands in for it.
  const node = selection.focusNode;
  const element = node instanceof Element ? node : node?.parentElement;
  if (!element) return null;
  const box = element.getBoundingClientRect();
  return { top: box.top, bottom: box.bottom };
}

/** Where the caret of the focused field inside `scroller` is, or null. */
export function caretBox(scroller: HTMLElement): CaretBox | null {
  const doc = scroller.ownerDocument;
  const active = doc.activeElement;
  if (!active || !scroller.contains(active)) return null;
  if (active instanceof HTMLTextAreaElement) return textareaCaret(active);
  if (active instanceof HTMLElement && active.isContentEditable) return selectionCaret(doc);
  return null;
}

/** Scrolls `scroller` until the focused field's caret line is in view. */
export function revealCaret(scroller: HTMLElement): void {
  const caret = caretBox(scroller);
  if (!caret) return;
  const view = scroller.getBoundingClientRect();
  const delta = caretScrollDelta(view, caret);
  if (delta) scroller.scrollTop += delta;
}

export function useComposerCaretFollow(scroller: Ref<HTMLElement | null>, enabled: () => boolean) {
  let frame = 0;

  function schedule() {
    if (!enabled() || frame) return;
    // After the editor and the textarea autosize have laid the new line out.
    frame = requestAnimationFrame(() => {
      frame = 0;
      const el = scroller.value;
      if (el && enabled()) revealCaret(el);
    });
  }

  // The keyboard rising shrinks the scroller from below, over the caret; on
  // iOS it does so in several steps as the suggestion bar settles.
  const resizes = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(schedule);

  watch(scroller, (el, previous) => {
    previous?.removeEventListener('input', schedule, true);
    if (previous) resizes?.unobserve(previous);
    el?.addEventListener('input', schedule, true);
    if (el) resizes?.observe(el);
  }, { immediate: true });

  watch(keyboardVisible, (up) => {
    if (up) schedule();
  });

  onBeforeUnmount(() => {
    scroller.value?.removeEventListener('input', schedule, true);
    resizes?.disconnect();
    if (frame) cancelAnimationFrame(frame);
  });

  return { reveal: schedule };
}
