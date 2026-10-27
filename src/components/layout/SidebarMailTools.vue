<script setup lang="ts">
/*
 * The sidebar's one action row: write a mail, and a small way into the
 * command palette. Search lives above the thread list and the account filter
 * in that list's filter menu, where they act on what they change.
 */
import { computed } from 'vue';
import { useUiStore } from '@/stores/ui';
import { ariaKeyshortcuts, bindingKeycaps } from '@/config/keybindings';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';

defineEmits<{ compose: [event: MouseEvent] }>();

const ui = useUiStore();

// The chord, not the rebindable `:` — the palette's own key is the one
// people know from other apps.
const paletteHint = computed(() => {
  const keys = bindingKeycaps('mod+k');
  return { label: keys.join(' '), keys, aria: ariaKeyshortcuts('mod+k') };
});
</script>

<template>
  <div class="sidebar-mail-tools">
    <AppButton
      variant="primary"
      class="sidebar-mail-tools__compose"
      :title="$t('nav.composeNewWindow')"
      @click="$emit('compose', $event)"
    >
      <template #icon><AppIcon><path d="M8 3v10M3 8h10" /></AppIcon></template>
      {{ $t('nav.newMessage') }}
    </AppButton>
    <IconButton
      size="lg"
      class="sidebar-mail-tools__palette"
      :title="$t('nav.commandPalette')"
      :shortcut="paletteHint"
      @click="ui.openOmni()"
    >
      <AppIcon><path d="M10 4v8a2 2 0 1 0 2-2H4a2 2 0 1 0 2 2V4a2 2 0 1 0-2 2h8a2 2 0 1 0-2-2" /></AppIcon>
    </IconButton>
  </div>
</template>

<style scoped>
.sidebar-mail-tools {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--space-1-5);
  padding: 0 var(--space-md) var(--space-5);
}

.sidebar-mail-tools__compose {
  justify-content: flex-start;
  min-width: 0;
  height: 38px;
  border-radius: var(--radius-md);
}

.sidebar-mail-tools__palette {
  border-radius: var(--radius-md);
  background: var(--bg-raised);
}
</style>
