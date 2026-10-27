<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import type { AnchorBox } from '@/utils/popoverPlacement';
import { sendLaterPresets } from '@/utils/sendLater';

const SendLaterPicker = defineAsyncComponent(() => import('./SendLaterPicker.vue'));
const props = defineProps<{ open: boolean; anchor: AnchorBox; value: string | null }>();
const emit = defineEmits<{ select: [value: string | null]; close: [] }>();
const { t, d } = useI18n();
const customOpen = ref(false);
const presets = computed(() => {
  // Recompute relative dates each time the menu opens, including after midnight.
  if (!props.open) return [];
  return sendLaterPresets().map(({ day, date }) => ({
    value: date.toISOString(),
    label: day === 'tomorrow'
      ? t('compose.sendLater.tomorrowAt', { time: d(date, 'time') })
      : t('compose.sendLater.mondayAt', { time: d(date, 'time') }),
  }));
});

async function pickCustom() {
  emit('close');
  // Let the menu restore focus to its trigger before the dialog captures it.
  await nextTick();
  customOpen.value = true;
}

function select(value: string | null) {
  customOpen.value = false;
  emit('select', value);
}
</script>

<template>
  <ContextMenu :open="open" :anchor="anchor" :label="t('compose.sendLater.title')" :width="272" :animated="false" @close="emit('close')">
    <div class="menu-group" role="group">
      <MenuItem v-for="preset in presets" :key="preset.value" icon="clock" :label="preset.label" @select="select(preset.value)" />
    </div>
    <div class="menu-group" role="group">
      <MenuItem icon="clock" :label="t('compose.sendLater.pickDateTime')" @select="pickCustom" />
      <MenuItem v-if="value" :label="t('compose.sendLater.clear')" @select="select(null)" />
    </div>
  </ContextMenu>
  <SendLaterPicker v-if="customOpen" :value="value" @select="select" @close="customOpen = false" />
</template>
