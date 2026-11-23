<script setup lang="ts">
/**
 * One row of a menu. Prefer a named glyph, so menus draw from one set; the
 * `icon` slot is for a glyph only one menu needs, a Lucide icon in AppIcon.
 * `trailing` holds a shortcut, the default slot a label with more than text.
 */
import { computed } from 'vue';
import {
  Bell, Bookmark, Check, ChevronRight, Clock, Inbox, Layers, Link, Mail, MailOpen, Newspaper, Pin, PinOff, Search, User, Zap,
  type IconNode,
} from 'lucide';
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

const MENU_ICONS: Record<MenuIcon, IconNode> = {
  check: Check,
  clock: Clock,
  aside: Bookmark,
  inbox: Inbox,
  pin: Pin,
  unpin: PinOff,
  'envelope-open': MailOpen,
  envelope: Mail,
  bolt: Zap,
  topic: Layers,
  person: User,
  bell: Bell,
  newsletter: Newspaper,
  search: Search,
  link: Link,
};

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
    <AppIcon v-else-if="props.icon" class="menu-item__icon" :icon="MENU_ICONS[props.icon]" />

    <span class="menu-item__label"><slot>{{ props.label }}</slot></span>

    <span v-if="$slots.trailing" class="menu-item__trailing"><slot name="trailing" /></span>
    <AppIcon v-if="props.checked" class="menu-item__state" :icon="Check" />
    <AppIcon v-else-if="props.submenu" class="menu-item__state menu-item__state--chevron" :icon="ChevronRight" />
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
