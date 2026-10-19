<script setup lang="ts">
/*
 * A button that is a region, not a pill.
 *
 * The kit had one button shape — `AppButton`, a centred pill — so every
 * surface whose click target is a card or a row (an attachment, a topic, a
 * folder tile) rolled its own `<button>` and reset the same five properties
 * to get back to a plain, left-aligned box. This is that reset, once. The
 * look belongs to the caller: this component contributes no padding, fill,
 * radius or border, only the behaviour and the reset.
 */
withDefaults(
  defineProps<{
    title?: string;
    disabled?: boolean;
    busy?: boolean;
  }>(),
  {
    title: undefined,
    disabled: false,
    busy: false,
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button
    type="button"
    class="card-button"
    :title="title"
    :disabled="disabled"
    :aria-busy="busy || undefined"
    @click="$emit('click', $event)"
  >
    <slot />
  </button>
</template>

<style scoped>
/* Caller utilities own layout and appearance; keep the reset below them. */
@layer components {
  .card-button {
    display: flex;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .card-button:disabled {
    cursor: default;
  }

  .card-button[aria-busy='true'] {
    cursor: progress;
  }
}
</style>
