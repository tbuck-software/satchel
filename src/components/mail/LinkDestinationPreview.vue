<script setup lang="ts">
import { computed } from 'vue';
import type { LinkAnchorRect } from '@/utils/externalLinks';

export type LinkPreviewStatus = 'hover' | 'error';

const props = defineProps<{
  href: string;
  status: LinkPreviewStatus;
  detail?: string;
  anchor: LinkAnchorRect;
}>();

const positionStyle = computed(() => {
  const viewportWidth = window.innerWidth;
  const estimatedWidth = Math.min(560, Math.max(180, props.href.length * 7.35 + 20));
  const left = Math.max(12, Math.min(props.anchor.left, viewportWidth - estimatedWidth - 12));
  const placeAbove = props.anchor.top > 48;
  return {
    left: `${left}px`,
    top: `${placeAbove ? props.anchor.top - 7 : props.anchor.bottom + 7}px`,
    transform: placeAbove ? 'translateY(-100%)' : undefined,
    maxWidth: `${Math.max(180, viewportWidth - 24)}px`,
  };
});
</script>

<template>
  <Teleport to="body">
    <Transition name="link-preview">
      <aside
        :class="['link-preview-card', `link-preview-card--${status}`]"
        :style="positionStyle"
        :aria-label="detail || $t('mail.link.checkTarget')"
        aria-live="polite"
      >
        <span v-if="status === 'error'" class="link-preview-icon" aria-hidden="true">!</span>
        <span class="link-preview-address">{{ href }}</span>
      </aside>
    </Transition>
  </Teleport>
</template>

<style scoped>
.link-preview-card {
  position: fixed;
  z-index: 900;
  display: flex;
  align-items: center;
  gap: 7px;
  width: fit-content;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, white 7%);
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--bg-overlay) 94%, transparent);
  padding: 6px 9px 7px;
  box-shadow: 0 7px 20px rgba(0, 0, 0, 0.34);
  backdrop-filter: blur(12px) saturate(125%);
  -webkit-backdrop-filter: blur(12px) saturate(125%);
  pointer-events: none;
}

.link-preview-icon {
  color: var(--accent-red);
  font-family: var(--font-sans);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
}

.link-preview-address {
  overflow: hidden;
  color: var(--text-primary);
  font-family: var(--font-mono);
  font-size: var(--font-size-xs);
  line-height: 1.35;
  overflow-wrap: anywhere;
}

.link-preview-card--error {
  border-color: color-mix(in srgb, var(--accent-red) 38%, var(--border-color));
}

.link-preview-enter-active,
.link-preview-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out);
}
.link-preview-enter-from,
.link-preview-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .link-preview-enter-active,
  .link-preview-leave-active { transition: none; }
}
</style>
