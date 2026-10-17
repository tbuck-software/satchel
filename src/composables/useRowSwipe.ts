import { computed, onBeforeUnmount, ref } from 'vue';
import { useSettingsStore } from '@/stores/settings';
import {
  swipeActionTone,
  type SwipeActionId,
  type SwipeSlot,
  type SwipeActionTone,
} from '@/utils/swipeActions';

export const SWIPE_LOCK_PX = 9;
export const SWIPE_SHORT_TRIGGER_PX = 72;
export const SWIPE_LONG_TRIGGER_PX = 156;
export const SWIPE_SOFT_LIMIT_PX = 184;
export const SWIPE_SHORT_COMMIT_OFFSET_PX = 104;
export const SWIPE_LONG_COMMIT_OFFSET_PX = 184;

/**
 * Shared row-swipe gesture: pointer tracking, axis locking, threshold
 * resolution against the configured per-slot actions, and the settle-then-
 * commit beat. Used by single thread rows and grouped-stack cards alike so
 * both surfaces stay one gesture vocabulary.
 */
export function useRowSwipe(options: {
  /** Extra guard for starting a gesture (leaving rows, selection mode, …). */
  enabled?: () => boolean;
  /** Label shown in the revealed track for the currently armed candidate. */
  actionLabel: (action: SwipeActionId | null) => string;
  /** Runs after the surface has settled onto its commit offset. */
  onCommit: (action: SwipeActionId) => void;
}) {
  const settings = useSettingsStore();

  const swipeOffset = ref(0);
  const swipeTracking = ref(false);
  const swipeSettling = ref(false);
  let swipePointerId: number | null = null;
  let swipeStartX = 0;
  let swipeStartY = 0;
  let swipeAxis: 'pending' | 'horizontal' | 'vertical' = 'pending';
  let suppressClickUntil = 0;
  let swipeCommitTimer: number | null = null;

  const isTouchSwiping = computed(() => swipeTracking.value || swipeSettling.value);
  const swipeSurfaceStyle = computed(() => ({
    transform: `translate3d(${swipeOffset.value}px, 0, 0)`,
  }));
  const swipeDirection = computed<'right' | 'left' | null>(() => {
    if (swipeOffset.value > 0) return 'right';
    if (swipeOffset.value < 0) return 'left';
    return null;
  });
  const swipeDistance = computed(() => Math.abs(swipeOffset.value));
  const swipeIsLong = computed(() => swipeDistance.value >= SWIPE_LONG_TRIGGER_PX);
  const swipeCandidateSlot = computed<SwipeSlot | null>(() => {
    if (swipeDirection.value === 'right') {
      return swipeIsLong.value ? 'rightLong' : 'rightShort';
    }
    if (swipeDirection.value === 'left') {
      return swipeIsLong.value ? 'leftLong' : 'leftShort';
    }
    return null;
  });
  const swipeCandidateAction = computed<SwipeActionId | null>(() => {
    const slot = swipeCandidateSlot.value;
    return slot ? settings.swipeActions[slot] : null;
  });
  const swipeAction = computed<SwipeActionId | null>(() => {
    if (swipeDistance.value < SWIPE_SHORT_TRIGGER_PX) return null;
    const candidate = swipeCandidateAction.value;
    return candidate && candidate !== 'none' ? candidate : null;
  });
  const swipeTone = computed<SwipeActionTone>(() =>
    swipeCandidateAction.value ? swipeActionTone(swipeCandidateAction.value) : 'neutral',
  );
  const swipeProgress = computed(() =>
    Math.min(1, swipeDistance.value / SWIPE_SHORT_TRIGGER_PX),
  );
  const swipeRevealStyle = computed(() => ({
    width: `${swipeDistance.value}px`,
    '--swipe-progress': swipeProgress.value,
  }));
  const swipeActionLabel = computed(() => options.actionLabel(swipeCandidateAction.value));

  function resistedSwipeOffset(delta: number): number {
    const sign = Math.sign(delta);
    const distance = Math.abs(delta);
    if (distance <= SWIPE_SOFT_LIMIT_PX) return delta;
    return sign * Math.min(224, SWIPE_SOFT_LIMIT_PX + (distance - SWIPE_SOFT_LIMIT_PX) * 0.18);
  }

  function resetSwipe() {
    swipeTracking.value = false;
    swipeSettling.value = false;
    swipePointerId = null;
    swipeAxis = 'pending';
    swipeOffset.value = 0;
  }

  function onSwipePointerDown(event: PointerEvent) {
    if (
      (event.pointerType !== 'touch' && event.pointerType !== 'pen')
      || event.button > 0
      || options.enabled?.() === false
    ) return;

    if (swipeCommitTimer !== null) window.clearTimeout(swipeCommitTimer);
    swipePointerId = event.pointerId;
    swipeStartX = event.clientX;
    swipeStartY = event.clientY;
    swipeAxis = 'pending';
    swipeTracking.value = true;
    swipeSettling.value = false;
  }

  function onSwipePointerMove(event: PointerEvent) {
    if (!swipeTracking.value || event.pointerId !== swipePointerId) return;
    const deltaX = event.clientX - swipeStartX;
    const deltaY = event.clientY - swipeStartY;

    if (swipeAxis === 'pending') {
      if (Math.max(Math.abs(deltaX), Math.abs(deltaY)) < SWIPE_LOCK_PX) return;
      swipeAxis = Math.abs(deltaX) > Math.abs(deltaY) * 1.15 ? 'horizontal' : 'vertical';
      if (swipeAxis === 'vertical') {
        resetSwipe();
        return;
      }
      try {
        (event.currentTarget as HTMLElement)?.setPointerCapture?.(event.pointerId);
      } catch {
        // Synthetic events and a pointer cancelled by the browser between the
        // lock decision and capture can legitimately reject capture. The
        // gesture can still finish through the bubbling pointer events.
      }
    }

    if (swipeAxis !== 'horizontal') return;
    event.preventDefault();
    suppressClickUntil = Date.now() + 450;
    swipeOffset.value = resistedSwipeOffset(deltaX);
  }

  function commitSwipe(action: SwipeActionId) {
    swipeTracking.value = false;
    swipeSettling.value = true;
    const direction = swipeDirection.value === 'left' ? -1 : 1;
    swipeOffset.value = direction * (
      swipeIsLong.value ? SWIPE_LONG_COMMIT_OFFSET_PX : SWIPE_SHORT_COMMIT_OFFSET_PX
    );
    suppressClickUntil = Date.now() + 600;
    swipeCommitTimer = window.setTimeout(() => {
      swipeCommitTimer = null;
      resetSwipe();
      options.onCommit(action);
    }, 110);
  }

  function onSwipePointerEnd(event: PointerEvent) {
    if (event.pointerId !== swipePointerId) return;
    if (swipeAxis === 'horizontal' && swipeAction.value) {
      commitSwipe(swipeAction.value);
      return;
    }
    resetSwipe();
  }

  function onSwipePointerCancel(event: PointerEvent) {
    if (event.pointerId === swipePointerId) resetSwipe();
  }

  function shouldSuppressClick(): boolean {
    return Date.now() < suppressClickUntil;
  }

  onBeforeUnmount(() => {
    if (swipeCommitTimer !== null) window.clearTimeout(swipeCommitTimer);
  });

  return {
    swipeTracking,
    swipeSettling,
    isTouchSwiping,
    swipeSurfaceStyle,
    swipeDirection,
    swipeDistance,
    swipeIsLong,
    swipeCandidateSlot,
    swipeCandidateAction,
    swipeAction,
    swipeTone,
    swipeRevealStyle,
    swipeActionLabel,
    onSwipePointerDown,
    onSwipePointerMove,
    onSwipePointerEnd,
    onSwipePointerCancel,
    shouldSuppressClick,
  };
}
