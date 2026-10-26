<script setup lang="ts">
import IconButton from '@/components/ui/IconButton.vue';
import { useUiStore } from '@/stores/ui';
import { useComposerStore } from '@/stores/composer';

/**
 * Writing a mail is an action, not a place.
 *
 * It sat in the bottom bar as a peer of the inbox, which reads as "another
 * screen you can be on" — and a tab you are never *in*, since the composer
 * opens over whatever you were doing. A floating button says what it is, and
 * puts it under the thumb that reaches the bottom-right corner most easily.
 */
const ui = useUiStore();
const composer = useComposerStore();

function compose() {
  ui.closeMobileSidebar();
  composer.openNew();
}
</script>

<template>
  <IconButton class="fab" :aria-label="$t('nav.composeMessage')" @click="compose" :title="$t('nav.composeMessage')">
    <svg width="24" height="24" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M12 1.5L14.5 4L5.5 13H3V10.5L12 1.5Z" />
      <path d="M10 3.5L12.5 6" />
    </svg>
  </IconButton>
</template>

<style scoped>
.fab {
  position: fixed;
  right: calc(20px + var(--satchel-safe-area-right));
  /* Clear of the gesture bar, not merely of the screen edge. */
  bottom: calc(20px + var(--satchel-safe-area-bottom));
  z-index: 60;
  display: flex;
  align-items: center;
  justify-content: center;
  /* The platform's FAB: a 56px circle, the same shape as the undo FAB that
     mirrors it in the other corner. ThreadGroupList's end inset is sized to
     clear this footprint. */
  width: 56px;
  height: 56px;
  border: 0;
  border-radius: var(--radius-full);
  color: var(--text-on-accent);
  background: var(--accent-primary);
  box-shadow: var(--shadow-lg);
  cursor: pointer;
  transition: transform var(--duration-fast) var(--ease-out);
}

.fab:active {
  transform: scale(0.94);
}
</style>
