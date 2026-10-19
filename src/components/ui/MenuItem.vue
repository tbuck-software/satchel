<script setup lang="ts">
/**
 * One row of a menu. Prefer a named glyph, so menus draw from one set; the
 * `icon` slot is for a glyph or marker only one menu needs, drawn in AppIcon.
 * `trailing` holds a shortcut, the default slot a label with more than text.
 */
import { computed } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';

export type MenuIcon =
  | 'check'
  | 'clock'
  | 'aside'
  | 'inbox'
  | 'pin'
  | 'unpin'
  | 'envelope-open'
  | 'envelope'
  | 'bolt'
  | 'topic'
  | 'person'
  | 'bell'
  | 'newsletter'
  | 'search'
  | 'link';

const props = withDefaults(
  defineProps<{
    label: string;
    icon?: MenuIcon | null;
    /**
     * Present makes the row checkable: it draws its state as a trailing
     * checkmark instead of the menu carrying a set/clear pair.
     */
    checked?: boolean;
    selection?: 'checkbox' | 'radio';
    disabled?: boolean;
    /** Opens a flyout; draws a trailing chevron. */
    submenu?: boolean;
    tone?: 'default' | 'danger';
    expanded?: boolean;
  }>(),
  { icon: null, checked: undefined, selection: 'checkbox', disabled: false, submenu: false, tone: 'default', expanded: false },
);

defineEmits<{ select: [event: MouseEvent] }>();

const role = computed(() => (props.checked === undefined ? 'menuitem' : `menuitem${props.selection}`));
</script>

<template>
  <button
    type="button"
    class="menu-item"
    :class="{
      'menu-item--checked': props.checked,
      'menu-item--danger': props.tone === 'danger',
      'menu-item--expanded': props.submenu && props.expanded,
    }"
    :role="role"
    :aria-checked="props.checked"
    :disabled="props.disabled"
    :aria-haspopup="props.submenu ? 'menu' : undefined"
    :aria-expanded="props.submenu ? props.expanded : undefined"
    @click="$emit('select', $event)"
  >
    <span v-if="$slots.icon" class="menu-item__icon"><slot name="icon" /></span>
    <AppIcon v-else-if="props.icon" class="menu-item__icon">
      <path v-if="props.icon === 'check'" d="M3.5 8.4 6.5 11.4 12.5 5" />
      <g v-else-if="props.icon === 'clock'"><circle cx="8" cy="8" r="5.6" /><path d="M8 4.8V8l2.3 1.4" /></g>
      <g v-else-if="props.icon === 'aside'"><path d="M2.6 9.4h3.1l.9 1.7h2.8l.9-1.7h3.1" /><path d="M3.6 9.4 5 4.6h6l1.4 4.8" /></g>
      <g v-else-if="props.icon === 'inbox'"><path d="M8 11V3.4M5.4 8.4 8 11l2.6-2.6" /><path d="M2.8 11.2v1.2a1 1 0 0 0 1 1h8.4a1 1 0 0 0 1-1v-1.2" /></g>
      <g v-else-if="props.icon === 'pin'"><path d="M6.4 2.4h3.2l-.45 4.3 2.45 2.2v1H4.4v-1l2.45-2.2z" /><path d="M8 9.9v3.7" /></g>
      <g v-else-if="props.icon === 'unpin'"><path d="M6.4 2.4h3.2l-.45 4.3 2.45 2.2v1H4.4v-1l2.45-2.2z" /><path d="M8 9.9v3.7" /><path d="M2.4 2.4 13.6 13.6" /></g>
      <g v-else-if="props.icon === 'envelope-open'"><path d="M2.8 7.2 8 3.2l5.2 4v5.2a1 1 0 0 1-1 1H3.8a1 1 0 0 1-1-1z" /><path d="m2.8 7.2 5.2 3.6 5.2-3.6" /></g>
      <g v-else-if="props.icon === 'envelope'"><rect x="2.6" y="3.8" width="10.8" height="8.4" rx="1.4" /><path d="m3 5 5 3.4L13 5" /></g>
      <path v-else-if="props.icon === 'bolt'" d="M9.2 1.8 4.4 8.4h3.4l-1 5.8 4.8-7H8.2z" />
      <g v-else-if="props.icon === 'topic'"><path d="M2.4 4.6c0-.8.7-1.5 1.5-1.5h2.6L8 4.6h4.1c.8 0 1.5.7 1.5 1.5v5.4c0 .8-.7 1.5-1.5 1.5H3.9c-.8 0-1.5-.7-1.5-1.5z" /><path d="M8 7.2v4M6 9.2h4" /></g>
      <g v-else-if="props.icon === 'link'"><path d="M6.8 9.2a2.6 2.6 0 0 0 3.7 0l2.1-2.1a2.6 2.6 0 0 0-3.7-3.7l-.9.9" /><path d="M9.2 6.8a2.6 2.6 0 0 0-3.7 0L3.4 8.9a2.6 2.6 0 0 0 3.7 3.7l.9-.9" /></g>
      <g v-else-if="props.icon === 'person'"><circle cx="8" cy="5.7" r="2.5" /><path d="M3.8 13a4.2 4.2 0 0 1 8.4 0" /></g>
      <g v-else-if="props.icon === 'bell'"><path d="M5 7.2a3 3 0 0 1 6 0c0 2 .8 2.8 1.4 3.4H3.6C4.2 10 5 9.2 5 7.2z" /><path d="M6.8 12.4a1.4 1.4 0 0 0 2.4 0" /></g>
      <g v-else-if="props.icon === 'newsletter'"><rect x="2.8" y="3.4" width="10.4" height="9.2" rx="1.2" /><path d="M5 6h6M5 8.4h6M5 10.8h3" /></g>
      <g v-else-if="props.icon === 'search'"><circle cx="7.2" cy="7.2" r="4.2" /><path d="m10.4 10.4 3 3" /></g>
    </AppIcon>

    <span class="menu-item__label"><slot>{{ props.label }}</slot></span>

    <span v-if="$slots.trailing" class="menu-item__trailing"><slot name="trailing" /></span>
    <AppIcon v-if="props.checked" class="menu-item__state">
      <path d="M3.5 8.4 6.5 11.4 12.5 5" />
    </AppIcon>
    <AppIcon v-else-if="props.submenu" class="menu-item__state menu-item__state--chevron">
      <path d="m6.2 3.4 5 4.6-5 4.6" />
    </AppIcon>
  </button>
</template>

<style scoped>
.menu-item {
  display: flex;
  align-items: center;
  gap: var(--space-2-5);
  width: 100%;
  padding: var(--space-1-5) var(--space-2-5);
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: var(--font-size-sm);
  text-align: left;
  cursor: pointer;
  transition:
    background var(--duration-fast) var(--ease-out),
    color var(--duration-fast) var(--ease-out);
}

.menu-item__icon {
  flex: 0 0 16px;
  width: 16px;
  height: 16px;
  color: var(--text-tertiary);
  transition: color var(--duration-fast) var(--ease-out);
}

.menu-item__label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.menu-item__icon {
  display: inline-grid;
  place-items: center;
}

.menu-item__trailing {
  display: inline-flex;
  align-items: center;
  color: var(--text-tertiary);
}

.menu-item__state {
  width: 14px;
  height: 14px;
  color: var(--accent-primary);
}

.menu-item__state--chevron {
  color: var(--text-tertiary);
}

.menu-item:disabled { opacity: 0.45; cursor: default; }

.menu-item:hover:not(:disabled),
.menu-item:focus-visible,
.menu-item--expanded {
  background: var(--bg-hover);
  color: var(--text-primary);
}

/* Hover and keyboard focus share the fill, so focus keeps the ring on top —
   in a menu it has to be readable which row the pointer is on and which the
   keyboard is on. */
.menu-item:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.menu-item:hover .menu-item__icon,
.menu-item:focus-visible .menu-item__icon {
  color: var(--text-secondary);
}

.menu-item--checked,
.menu-item--checked .menu-item__icon {
  color: var(--text-primary);
}

.menu-item--danger {
  color: var(--accent-red);
}

.menu-item--danger .menu-item__icon {
  color: color-mix(in srgb, var(--accent-red) 80%, transparent);
}

.menu-item--danger:hover,
.menu-item--danger:focus-visible {
  background: color-mix(in srgb, var(--accent-red) 12%, transparent);
  color: var(--accent-red);
}

.menu-item--danger:hover .menu-item__icon {
  color: var(--accent-red);
}
</style>
