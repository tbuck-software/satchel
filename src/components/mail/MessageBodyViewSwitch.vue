<script setup lang="ts">
/**
 * Stands where EmailRenderer draws its Original / Auto-dark / Text switch
 * (see bodyViewSwitch.ts). The desktop keeps the segmented control. The phone
 * moves the choice into the message's ⋯ sheet, which reads it from the state
 * reported here; above the body it leaves one muted line, and only while the
 * mail is not shown the way it opened: what it is shown as, and the way back.
 */
import { computed, inject, onBeforeUnmount, watchEffect } from 'vue';
import { useI18n } from 'vue-i18n';
import AppButton from '@/components/ui/AppButton.vue';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';
import { useUiStore } from '@/stores/ui';
import { BODY_VIEW_STATE, type BodyViewOption } from './bodyViewState';

const props = defineProps<{
  options: BodyViewOption[];
  modelValue: string;
  defaultValue: string;
  ariaLabel?: string;
}>();
const emit = defineEmits<{ 'update:modelValue': [value: string] }>();

const ui = useUiStore();
const { t } = useI18n();
const state = inject(BODY_VIEW_STATE, null);

function choose(value: string) {
  emit('update:modelValue', value);
}

watchEffect(() => {
  if (!state) return;
  state.value = {
    options: props.options,
    mode: props.modelValue,
    defaultMode: props.defaultValue,
    set: choose,
  };
});
onBeforeUnmount(() => {
  if (state) state.value = null;
});

const shownAs = computed(() => {
  switch (props.modelValue) {
    case 'html': return t('mail.viewMode.shownAs.original');
    case 'auto-dark': return t('mail.viewMode.shownAs.autoDark');
    case 'markdown': return t('mail.viewMode.shownAs.markdown');
    default: return t('mail.viewMode.shownAs.text');
  }
});

const defaultOption = computed(() => props.options.find((option) => option.value === props.defaultValue));
</script>

<template>
  <SegmentedControl
    v-if="!ui.isMobile"
    :options="props.options"
    :model-value="props.modelValue"
    :aria-label="props.ariaLabel"
    @update:model-value="choose"
  />
  <p v-else-if="defaultOption && props.modelValue !== props.defaultValue" class="body-view-note">
    <span>{{ shownAs }}</span>
    <span aria-hidden="true">·</span>
    <AppButton
      class="body-view-note__back"
      :aria-label="t('mail.viewMode.backTo', { mode: defaultOption.label })"
      variant="link"
      size="sm"
      @click="choose(defaultOption.value)"
    >
      {{ defaultOption.label }}
    </AppButton>
  </p>
</template>

<style scoped>
/* A fact and a way out, in the image notice's quiet register: no box,
   tertiary ink, and the one action in the accent. */
.body-view-note {
  display: flex;
  align-items: baseline;
  gap: var(--space-xs);
  margin: 0 0 var(--space-sm);
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}

.body-view-note__back {
  color: var(--text-accent);
  font-weight: var(--weight-medium);
}
</style>
