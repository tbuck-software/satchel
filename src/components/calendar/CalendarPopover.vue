<script setup lang="ts">
import { useDismissable } from '@/composables/useDismissable';
import { useUiStore } from '@/stores/ui';
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import {
  placePopoverBesideAnchor,
  type AnchorBox,
  type SidePlacement,
} from '@/utils/popoverPlacement';

// `aria-label` falls through to the root element via $attrs.
const props = withDefaults(
  defineProps<{
    anchor: AnchorBox | null;
    width?: number;
  }>(),
  { width: 320 },
);

const emit = defineEmits<{
  close: [];
}>();

const ui = useUiStore();
const card = ref<HTMLElement | null>(null);
const placement = ref<SidePlacement | null>(null);
let resizeObserver: ResizeObserver | null = null;

function reposition() {
  // A sheet is pinned to the bottom edge; there is nothing to place beside.
  if (ui.isSinglePane) return;
  if (!props.anchor || !card.value) return;
  placement.value = placePopoverBesideAnchor(
    props.anchor,
    { width: window.innerWidth, height: window.innerHeight },
    { width: props.width, height: card.value.offsetHeight },
  );
}

watch(
  () => props.anchor,
  (anchor) => {
    if (!anchor) {
      placement.value = null;
      return;
    }
    // Render invisible first, measure the real height, then place. Focus must
    // wait until the placement style is applied: a visibility:hidden element
    // silently refuses focus, which left the quick-create title field dead.
    void nextTick(() => {
      reposition();
      void nextTick(() => {
        card.value?.querySelector<HTMLElement>('[autofocus]')?.focus();
      });
    });
  },
  { immediate: true },
);

function onPointerdown(event: PointerEvent) {
  if (!props.anchor) return;
  if (card.value && !card.value.contains(event.target as Node)) emit('close');
}

function onKeydown(event: KeyboardEvent) {
  if (props.anchor && event.key === 'Escape') {
    event.preventDefault();
    emit('close');
  }
}

// The grid scrolls underneath a fixed-position card; closing is the honest
// response, the same call every mainstream calendar makes.
function onScroll(event: Event) {
  if (!props.anchor) return;
  if (card.value && event.target instanceof Node && card.value.contains(event.target)) return;
  emit('close');
}

onMounted(() => {
  window.addEventListener('pointerdown', onPointerdown, true);
  window.addEventListener('keydown', onKeydown);
  window.addEventListener('scroll', onScroll, true);
  window.addEventListener('resize', reposition);
  resizeObserver = new ResizeObserver(() => reposition());
  watch(
    card,
    (el, _prev, onCleanup) => {
      if (el) {
        resizeObserver?.observe(el);
        onCleanup(() => resizeObserver?.unobserve(el));
      }
    },
    { immediate: true },
  );
});

onBeforeUnmount(() => {
  window.removeEventListener('pointerdown', onPointerdown, true);
  window.removeEventListener('keydown', onKeydown);
  window.removeEventListener('scroll', onScroll, true);
  window.removeEventListener('resize', reposition);
  resizeObserver?.disconnect();
});

// Back closes this before whatever it was opened from.
useDismissable(() => props.anchor !== null, () => emit('close'));
</script>

<template>
  <Teleport to="body">
    <Transition name="cal-popover">
      <div
        v-if="anchor"
        ref="card"
        class="cal-popover"
        role="dialog"
        tabindex="-1"
        :class="{ 'cal-popover--sheet': ui.isSinglePane }"
        :style="ui.isSinglePane
          ? undefined
          : {
              width: `${width}px`,
              visibility: placement ? 'visible' : 'hidden',
              left: placement?.left,
              top: placement?.top,
              maxHeight: placement?.maxHeight,
            }"
      >
        <slot />
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* A popover anchored near the top of a phone screen is a stretch away and
   often lands half off it. Pinned to the bottom edge instead, where the thumb
   already is — and no longer positioned by script, so nothing fights it. */
.cal-popover--sheet {
  inset: auto 0 0 0;
  width: 100%;
  max-height: 70vh;
  padding-right: var(--satchel-safe-area-right);
  padding-bottom: var(--satchel-safe-area-bottom);
  padding-left: var(--satchel-safe-area-left);
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  visibility: visible;
}

.cal-popover {
  position: fixed;
  z-index: 900;
  display: flex;
  flex-direction: column;
  border: var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  box-shadow: var(--shadow-lg);
  overflow: auto;
}

.cal-popover-enter-active,
.cal-popover-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out),
    transform var(--duration-fast) var(--ease-out);
}

.cal-popover-enter-from,
.cal-popover-leave-to {
  opacity: 0;
  transform: translateY(4px) scale(0.99);
}

@media (prefers-reduced-motion: reduce) {
  .cal-popover-enter-active,
  .cal-popover-leave-active {
    transition: none;
  }
}
</style>
