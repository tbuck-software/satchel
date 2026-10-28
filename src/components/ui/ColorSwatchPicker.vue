<script setup lang="ts">
/**
 * A row of colour swatches, one of them chosen. On its own it is a radio
 * group with arrow keys between swatches; inside a ContextMenu (`menu`) the
 * swatches are menu radio items, so the menu's own arrow keys reach them.
 *
 * `customLabel` adds a last swatch that opens the system colour picker.
 * Swatch colours are values the caller stores, not theme colours.
 */
import { computed, ref } from 'vue';

const props = withDefaults(
  defineProps<{
    modelValue: string;
    options: ReadonlyArray<{ value: string; label: string }>;
    label: string;
    menu?: boolean;
    customLabel?: string | null;
    disabled?: boolean;
  }>(),
  { menu: false, customLabel: null, disabled: false },
);

const emit = defineEmits<{ 'update:modelValue': [value: string] }>();

const root = ref<HTMLElement | null>(null);
const current = computed(() => props.modelValue.toLowerCase());
const isCustom = computed(() => !props.options.some((option) => option.value.toLowerCase() === current.value));
const focusIndex = computed(() => Math.max(0, props.options.findIndex((option) => option.value.toLowerCase() === current.value)));

function choose(value: string) {
  if (props.disabled || value.toLowerCase() === current.value) return;
  emit('update:modelValue', value.toLowerCase());
}

function onKeydown(event: KeyboardEvent) {
  const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
  if (!step) return;
  const swatches = Array.from(root.value?.querySelectorAll<HTMLElement>('.color-swatch') ?? []);
  const index = swatches.indexOf(document.activeElement as HTMLElement);
  if (index < 0) return;
  event.preventDefault();
  event.stopPropagation();
  const next = swatches[(index + step + swatches.length) % swatches.length];
  next?.focus();
  // A radio group follows focus; a menu waits for Enter like its other items.
  if (!props.menu && next?.dataset.value) choose(next.dataset.value);
}
</script>

<template>
  <div
    ref="root"
    class="color-swatches"
    :role="menu ? 'group' : 'radiogroup'"
    :aria-label="label"
    :aria-disabled="disabled || undefined"
    @keydown="onKeydown"
  >
    <button
      v-for="(option, index) in options"
      :key="option.value"
      type="button"
      class="color-swatch"
      :class="{ 'color-swatch--checked': option.value.toLowerCase() === current }"
      :style="{ '--swatch-color': option.value }"
      :role="menu ? 'menuitemradio' : 'radio'"
      :aria-checked="option.value.toLowerCase() === current"
      :aria-label="option.label"
      :title="option.label"
      :tabindex="menu ? -1 : index === focusIndex && !isCustom ? 0 : -1"
      :disabled="disabled"
      :data-value="option.value"
      @click="choose(option.value)"
    />
    <label
      v-if="customLabel && !menu"
      class="color-swatch color-swatch--custom"
      :class="{ 'color-swatch--checked': isCustom }"
      :style="isCustom ? { '--swatch-color': current } : undefined"
      :title="customLabel"
    >
      <input
        type="color"
        class="color-swatch__input"
        :value="current"
        :aria-label="customLabel"
        :tabindex="isCustom ? 0 : -1"
        :disabled="disabled"
        @change="choose(($event.target as HTMLInputElement).value)"
      />
    </label>
  </div>
</template>

<style scoped>
.color-swatches {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-xs);
}

.color-swatch {
  position: relative;
  width: 22px;
  height: 22px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-full);
  background: var(--swatch-color);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--text-primary) 12%, transparent);
  cursor: pointer;
  transition: transform var(--duration-fast) var(--ease-out);
}

.color-swatch:hover:not(:disabled) {
  transform: scale(1.08);
}

.color-swatch:disabled {
  cursor: default;
  opacity: 0.5;
}

/* Chosen: a ring of the surface it sits on, then one of the swatch's own.
   Callers on another surface set --swatch-ring to it. */
.color-swatch--checked {
  box-shadow:
    0 0 0 2px var(--swatch-ring, var(--bg-surface)),
    0 0 0 4px var(--swatch-color);
}

.color-swatch--custom {
  overflow: hidden;
  background: conic-gradient(
    var(--accent-red),
    var(--accent-amber),
    var(--accent-green),
    var(--accent-cyan),
    var(--accent-primary),
    var(--accent-purple),
    var(--accent-pink),
    var(--accent-red)
  );
}

.color-swatch--custom.color-swatch--checked {
  background: var(--swatch-color);
}

.color-swatch--custom:focus-within {
  outline: 2px solid var(--accent-primary);
  outline-offset: 2px;
}

/* The native control stays the click and keyboard target, but invisible:
   its own chrome differs per engine and would not match the round swatch. */
.color-swatch__input {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  opacity: 0;
  cursor: pointer;
}
</style>
