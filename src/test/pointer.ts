import { nextTick } from 'vue';

/**
 * Dispatches a real `PointerEvent`, bubbling and cancelable like `trigger`.
 *
 * `trigger` cannot: after constructing the event it assigns every option
 * onto it again, and jsdom 30's `PointerEvent` inherits read-only `clientX`
 * and `button` from `MouseEvent`, so the assignment throws.
 */
export async function pointer(
  target: { element: Element },
  type: string,
  init: PointerEventInit = {},
): Promise<void> {
  target.element.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, ...init }));
  await nextTick();
}
