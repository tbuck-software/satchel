<script setup lang="ts">
import { nextTick } from 'vue';

const props = withDefaults(
  defineProps<{
    modelValue: boolean;
    disabled?: boolean;
    /** The name for a switch whose caption sits outside it, in the row. */
    label?: string;
    /**
     * Asked before the switch moves; returning false keeps it where it was.
     *
     * For the flips someone else can refuse — Android may deny the
     * notification permission, and a switch that slid over anyway would be
     * lying about the state of the system.
     */
    beforeChange?: (next: boolean) => boolean | Promise<boolean>;
  }>(),
  {
    disabled: false,
    label: undefined,
    beforeChange: undefined,
  },
);

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

async function onChange(event: Event) {
  const input = event.target as HTMLInputElement;
  const next = input.checked;

  if (!props.beforeChange || await props.beforeChange(next)) {
    emit('update:modelValue', next);
  }

  // The checkbox has already moved itself. Vue only patches `checked` back
  // when the bound value changed, so a refused — or simply ignored — flip
  // would leave the knob standing on a state nobody holds.
  await nextTick();
  input.checked = props.modelValue;
}
</script>

<template>
  <label
    class="toggle-switch"
    :class="{ 'toggle-switch--disabled': disabled, 'toggle-switch--bare': !$slots.default }"
  >
    <input
      type="checkbox"
      role="switch"
      class="toggle-switch__input"
      :aria-label="label"
      :checked="modelValue"
      :disabled="disabled"
      @change="onChange"
    />
    <span class="toggle-switch__track" aria-hidden="true">
      <span class="toggle-switch__thumb" />
    </span>
    <span v-if="$slots.default" class="toggle-switch__label">
      <slot />
    </span>
  </label>
</template>

<style scoped>
.toggle-switch {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2-5);
  min-width: 0;
  cursor: pointer;
  user-select: none;
}

/*
 * Without a caption there is nothing to truncate. In a settings row a long
 * hint would otherwise squeeze the label below its track, and the track
 * would overflow past the row's right inset.
 */
.toggle-switch--bare {
  flex-shrink: 0;
}

.toggle-switch--disabled {
  cursor: default;
  opacity: 0.55;
}

.toggle-switch__input {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: 0;
  padding: 0;
  border: 0;
  clip-path: inset(50%);
  overflow: hidden;
  white-space: nowrap;
}

.toggle-switch__track {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
  box-sizing: border-box;
  width: 36px;
  height: 21px;
  padding: var(--space-2xs);
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
  box-shadow: inset 0 0 0 1px var(--border-color-muted);
  transition: background var(--duration-normal) var(--ease-out);
}

.toggle-switch__thumb {
  width: 17px;
  height: 17px;
  border-radius: 50%;
  background: var(--text-on-accent);
  box-shadow: var(--shadow-sm);
  transition: transform var(--duration-normal) var(--ease-out);
}

.toggle-switch__input:checked + .toggle-switch__track {
  background: var(--accent-primary);
  box-shadow: none;
}

.toggle-switch__input:checked + .toggle-switch__track .toggle-switch__thumb {
  transform: translateX(15px);
}

/* The input is clipped to a pixel, so the ring has to be drawn on the track. */
.toggle-switch__input:focus-visible + .toggle-switch__track {
  box-shadow: var(--focus-ring);
}

.toggle-switch__label {
  min-width: 0;
  font-size: var(--font-size-md);
  color: var(--text-primary);
}
</style>
