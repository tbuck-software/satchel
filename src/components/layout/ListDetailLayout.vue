<script setup lang="ts">
import { computed, onBeforeUnmount } from 'vue';
import { useUiStore } from '@/stores/ui';

/**
 * The list/detail split, resolved by available width.
 *
 * Wide enough, it is the inbox's shell: list pane on the left at the shared
 * thread-list width, a drag handle, detail on the right — so resizing feels
 * identical across every view that renders "rows of things with one selected".
 *
 * Too narrow for two panes, they become two screens: the list until something
 * is selected, the detail after. That makes selecting an entry a step you can
 * come back from, which is what the system back gesture needs to have anything
 * to return to. The platform never enters into it; only the width does, so a
 * narrow browser window behaves exactly like a phone.
 *
 * Modelled on Android's SlidingPaneLayout and the Material 3
 * ListDetailPaneScaffold, for the reason those exist: without it every screen
 * grows its own idea of what back means, and they disagree.
 */
const props = withDefaults(
  defineProps<{
    /**
     * True when a detail is selected. Left undefined by callers that always
     * have one, which keeps the old two-pane behaviour on wide screens and
     * simply never collapses to the list.
     */
    hasDetail?: boolean;
    /** Overrides the shared thread-list width for panes that are not lists of mail. */
    listWidth?: string;
  }>(),
  { hasDetail: true, listWidth: undefined },
);

const ui = useUiStore();

/** Narrow: exactly one pane is on screen, and the detail wins when present. */
const showList = computed(() => !ui.isSinglePane || !props.hasDetail);
const showDetail = computed(() => !ui.isSinglePane || props.hasDetail);

let resizeStartX = 0;
let resizeStartWidth = 0;

function onResizeMove(event: MouseEvent) {
  ui.setThreadListWidth(resizeStartWidth + (event.clientX - resizeStartX));
}

function stopResize() {
  document.body.style.cursor = '';
  document.body.style.userSelect = '';
  window.removeEventListener('mousemove', onResizeMove);
  window.removeEventListener('mouseup', stopResize);
}

function startResize(event: MouseEvent) {
  resizeStartX = event.clientX;
  resizeStartWidth = ui.threadListWidth;
  document.body.style.cursor = 'col-resize';
  document.body.style.userSelect = 'none';
  window.addEventListener('mousemove', onResizeMove);
  window.addEventListener('mouseup', stopResize);
}

onBeforeUnmount(stopResize);
</script>

<template>
  <section
    class="split-layout"
    :class="{ 'split-layout--single': ui.isSinglePane }"
    :style="{ '--list-w': listWidth ?? `${ui.threadListWidth}px` }"
  >
    <!-- Preserve list scroll and controls while the detail covers it. -->
    <article v-show="showList" class="split-layout__list">
      <slot name="list" />
    </article>
    <!-- A handle for a pane that is not beside anything would drag nothing. -->
    <div
      v-if="!ui.isSinglePane"
      class="split-layout__resize"
      role="separator"
      :aria-label="$t('ui.resizeList')"
      @mousedown.prevent="startResize"
    />
    <article v-if="showDetail" class="split-layout__detail">
      <!-- Nothing selected on a wide screen: the pane stays, empty, rather than
           the layout collapsing and shifting the list under the pointer. -->
      <slot v-if="hasDetail" name="detail" />
      <slot v-else name="empty" />
    </article>
  </section>
</template>

<style scoped>
.split-layout {
  height: 100%;
  display: grid;
  grid-template-columns: var(--list-w) 1px minmax(0, 1fr);
  overflow: hidden;
}

.split-layout--single {
  grid-template-columns: minmax(0, 1fr);
}

.split-layout--single .split-layout__list {
  border-right: 0;
}

.split-layout__list {
  min-height: 0;
  min-width: 0;
  overflow: hidden;
  display: grid;
  background: var(--bg-base);
  border-right: 1px solid var(--border-color-muted);
}

.split-layout__resize {
  width: 1px;
  background: var(--border-color-muted);
  cursor: col-resize;
  position: relative;
  z-index: 2;
}

.split-layout__resize::before {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  left: -3px;
  width: 7px;
}

.split-layout__resize:hover,
.split-layout__resize:active {
  background: var(--accent-primary);
  box-shadow: 0 0 8px color-mix(in srgb, var(--accent-primary) 30%, transparent);
}

.split-layout__detail {
  min-height: 0;
  min-width: 0;
  overflow: hidden;
  background: var(--bg-surface, var(--bg-base));
}
</style>
