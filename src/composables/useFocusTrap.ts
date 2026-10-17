import { nextTick, onBeforeUnmount, onMounted, watch, type Ref } from 'vue';

/**
 * The three halves of a keyboard-correct overlay: focus moved in, kept in, and
 * handed back to whatever opened it.
 *
 * Escape is deliberately absent. It runs through `ui.dismissTopmostOverlay()`,
 * which knows the nesting order; a second handler per overlay would close the
 * wrong surface whenever two are open.
 *
 * Most overlays used to get none of the three: the dialog appeared, the focus
 * stayed in the list behind it, and closing left it on `<body>` with the whole
 * page to tab through again.
 */

const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[contenteditable]:not([contenteditable="false"])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');

export interface FocusTrapOptions {
  /**
   * Reads whether the overlay is on screen. Overlays that their parent mounts
   * only while open pass nothing.
   */
  isOpen?: () => boolean;
  /**
   * Skip moving focus in. A surface a finger opened has no business pulling the
   * on-screen keyboard up with it; the focus is still handed back on the way
   * out.
   */
  moveFocusIn?: () => boolean;
  /**
   * Where focus rests when nothing inside asked for it. A named dialog wants
   * its own container, so the name is read out before the contents; a panel
   * that is nothing but a list of choices wants the first of them.
   */
  restingFocus?: 'container' | 'first';
}

export interface FocusTrap {
  /** Bind to `keydown.tab` on the overlay's outermost element. */
  onTab: (event: KeyboardEvent) => void;
  /** Focusable descendants in tab order — for tests and for callers that need
   *  to reach a particular one. */
  focusables: () => HTMLElement[];
}

export function useFocusTrap(
  container: Ref<HTMLElement | null>,
  options: FocusTrapOptions = {},
): FocusTrap {
  let previouslyFocused: HTMLElement | null = null;

  function focusables(): HTMLElement[] {
    return Array.from(container.value?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? []).filter(
      (element) => {
        if (element.matches(':disabled, input[type="hidden"]') || element.closest('[inert], [hidden]')) return false;
        if (element.hasAttribute('tabindex') && element.tabIndex < 0) return false;
        const visibility = getComputedStyle(element).visibility;
        if (visibility === 'hidden' || visibility === 'collapse') return false;
        // Layout geometry is unavailable in detached/test documents; inspect
        // ancestors so controls inside v-show panels are excluded as well.
        for (let parent: HTMLElement | null = element; parent; parent = parent.parentElement) {
          const style = getComputedStyle(parent);
          if (style.display === 'none' || style.contentVisibility === 'hidden') return false;
        }
        return true;
      },
    );
  }

  function onTab(event: KeyboardEvent) {
    // A field that already answered Tab keeps it: committing a recipient chip
    // or jumping to the next result group is not a request to move on.
    if (event.defaultPrevented) return;
    const items = focusables();
    event.preventDefault();
    if (items.length === 0) {
      container.value?.focus();
      return;
    }
    // -1 means focus escaped the container; Tab brings it back to the near end.
    const current = items.indexOf(document.activeElement as HTMLElement);
    const next = event.shiftKey
      ? current <= 0
        ? items.length - 1
        : current - 1
      : current === items.length - 1
        ? 0
        : current + 1;
    items[next]?.focus();
  }

  function remember() {
    if (typeof document === 'undefined') return;
    previouslyFocused = document.activeElement as HTMLElement | null;
  }

  function focusIn() {
    if (options.moveFocusIn && !options.moveFocusIn()) return;
    // A field that asked for focus gets it; otherwise wherever the overlay
    // wants focus to rest.
    const items = focusables();
    const resting = options.restingFocus === 'container' ? undefined : items[0];
    const target =
      items.find((item) => item.hasAttribute('autofocus')) ?? resting ?? container.value;
    target?.focus();
  }

  function leave() {
    if (typeof document === 'undefined' || !previouslyFocused) return;
    // Something that deliberately took focus on the way out — an editor after
    // an insert — keeps it. Otherwise it goes back where it came from.
    const active = document.activeElement;
    const movedOn = active && active !== document.body && !container.value?.contains(active);
    if (!movedOn) previouslyFocused.focus();
    previouslyFocused = null;
  }

  if (options.isOpen) {
    watch(
      options.isOpen,
      (open) => {
        if (!open) {
          leave();
          return;
        }
        remember();
        // The overlay renders in response to the same flag; it is not there yet.
        void nextTick(focusIn);
      },
      { immediate: true },
    );
  } else {
    // Captured in setup, before the overlay exists to take focus itself.
    remember();
    // Mounted means rendered, so this needs no tick: a caller may read the
    // focus straight after mounting.
    onMounted(focusIn);
  }

  onBeforeUnmount(leave);

  return { onTab, focusables };
}
