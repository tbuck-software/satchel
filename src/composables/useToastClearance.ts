import { onBeforeUnmount, onMounted, ref, shallowReactive, type Ref } from 'vue';

/**
 * Owners register the whole composer, the reply bar and the draft card. The
 * lower-left toast stack measures their bounds when notifications or layout
 * change, then clears them.
 */

export interface ClearanceRect {
  top: number;
  right: number;
  bottom: number;
  left: number;
}

const obstacles = shallowReactive(new Set<Ref<HTMLElement | null>>());

/** Bumped when a registered surface moved without the stack noticing. */
export const toastClearanceRevision = ref(0);

export function refreshToastClearance(): void {
  toastClearanceRevision.value += 1;
}

/**
 * Whether something other than the surface — or the stack itself, which may
 * be lying on it right now — is on top of the surface's centre. A reply bar
 * under a sheet or a dialog cannot be reached anyway, and lifting the stack
 * over it only put the toasts on whatever covers it: the close button of the
 * message details sheet, once the bar followed a short thread up the pane.
 */
function isCovered(element: HTMLElement, rect: DOMRect, stack: Element | null): boolean {
  if (typeof document.elementFromPoint !== 'function') return false;
  const top = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
  return top !== null && !element.contains(top) && !stack?.contains(top);
}

/** The registered surfaces that are on screen, and not under something, right now. */
export function toastObstacleRects(stack: Element | null = null): ClearanceRect[] {
  const rects: ClearanceRect[] = [];
  for (const target of obstacles) {
    const element = target.value;
    const rect = element?.getBoundingClientRect();
    if (!element || !rect || rect.width <= 0 || rect.height <= 0) continue;
    if (isCovered(element, rect, stack)) continue;
    rects.push(rect);
  }
  return rects;
}

/** Keeps the toast stack off `target` while it is mounted. */
export function useToastClearance(target: Ref<HTMLElement | null>): void {
  onMounted(() => {
    obstacles.add(target);
    refreshToastClearance();
  });
  onBeforeUnmount(() => {
    obstacles.delete(target);
    refreshToastClearance();
  });
}

function overlaps(a: ClearanceRect, b: ClearanceRect): boolean {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

/**
 * How far a stack at `stack` has to rise so it overlaps none of `rects`,
 * keeping `gap` to the one it ends up above. Lifting clear of one surface can
 * land the stack on another (the reply bar under an open composer), hence the
 * loop; each pass clears at least one more surface, so it ends.
 */
export function clearanceAbove(stack: ClearanceRect, rects: readonly ClearanceRect[], gap: number): number {
  let lift = 0;
  for (let pass = 0; pass <= rects.length; pass += 1) {
    const current = { ...stack, top: stack.top - lift, bottom: stack.bottom - lift };
    const blocking = rects.filter((rect) => overlaps(current, rect));
    if (blocking.length === 0) return lift;
    lift = Math.max(lift, ...blocking.map((rect) => stack.bottom - rect.top + gap));
  }
  return lift;
}
