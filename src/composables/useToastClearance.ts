import { onBeforeUnmount, onMounted, ref, shallowReactive, type Ref } from 'vue';

/**
 * Surfaces a toast must not cover: the composer's send row and the reader's
 * reply bar. The stack sits fixed in the bottom-right corner, which is exactly
 * where both of them end — and an error toast stays for nine seconds while a
 * failing sync keeps putting one back, so the stack used to sit on the Send
 * button that would have resolved the error.
 *
 * The owner registers its element; `ToastStack` measures the registered
 * elements and lifts itself above whichever it would overlap. Measured, not
 * configured: the composer is dragged, resized and maximised, the reply bar
 * comes and goes with the thread, and a fixed offset would be wrong for most
 * of those.
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

/** The registered surfaces that are on screen right now. */
export function toastObstacleRects(): ClearanceRect[] {
  const rects: ClearanceRect[] = [];
  for (const target of obstacles) {
    const rect = target.value?.getBoundingClientRect();
    if (rect && rect.width > 0 && rect.height > 0) rects.push(rect);
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
