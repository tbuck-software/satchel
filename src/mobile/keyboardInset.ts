/**
 * Keeps the on-screen keyboard out of the layout.
 *
 * iOS lays the keyboard over the page: the layout viewport keeps its height,
 * only `visualViewport` shrinks, and everything docked to the bottom — Send,
 * Save, a sheet's buttons — ends up underneath it. Android does the same when
 * the WebView runs edge to edge and `interactive-widget` is not honoured.
 *
 * The keyboard is system UI at the bottom edge, which is exactly what
 * `--satchel-safe-area-bottom` already describes. While it is up, this writes
 * its height into that property on `<html>`; the shell, the composer, sheets
 * and dialogs already pad themselves by it, so they shrink to the visible area
 * without knowing about keyboards. When it goes down the property is removed
 * and `env(safe-area-inset-bottom)` from base.css applies again.
 */

/** Anything smaller is a browser toolbar or rounding, not a keyboard. */
const MIN_KEYBOARD_PX = 80;

const PROPERTY = '--satchel-safe-area-bottom';
/** The keyboard alone, for what should stay behind it rather than ride on it. */
const KEYBOARD_PROPERTY = '--satchel-keyboard-inset';

export interface KeyboardViewport {
  height: number;
  offsetTop: number;
  scale: number;
}

/** Height the keyboard covers at the bottom of the layout viewport, or 0. */
export function keyboardInset(layoutHeight: number, viewport: KeyboardViewport): number {
  // Pinch zoom shrinks the visual viewport too; that is not a keyboard.
  if (viewport.scale > 1.01) return 0;
  const covered = Math.round(layoutHeight - viewport.offsetTop - viewport.height);
  return covered >= MIN_KEYBOARD_PX ? covered : 0;
}

export function installKeyboardInset(win: Window = window): () => void {
  const viewport = win.visualViewport;
  if (!viewport) return () => {};
  const root = win.document.documentElement;
  let applied = 0;

  const update = () => {
    const inset = keyboardInset(win.innerHeight, viewport);
    if (inset === applied) return;
    applied = inset;
    if (inset) {
      root.style.setProperty(PROPERTY, `${inset}px`);
      root.style.setProperty(KEYBOARD_PROPERTY, `${inset}px`);
      root.dataset.keyboard = 'open';
    } else {
      root.style.removeProperty(PROPERTY);
      root.style.removeProperty(KEYBOARD_PROPERTY);
      delete root.dataset.keyboard;
    }
    // iOS scrolls the whole page to bring a focused field into view. The
    // shell has already shrunk around the field, so that scroll only pushes
    // the header off the top.
    if (win.scrollY) win.scrollTo(0, 0);
  };

  viewport.addEventListener('resize', update);
  viewport.addEventListener('scroll', update);
  update();
  return () => {
    viewport.removeEventListener('resize', update);
    viewport.removeEventListener('scroll', update);
    root.style.removeProperty(PROPERTY);
    root.style.removeProperty(KEYBOARD_PROPERTY);
    delete root.dataset.keyboard;
  };
}
