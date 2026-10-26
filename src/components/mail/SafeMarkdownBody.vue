<script setup lang="ts">
import { computed, nextTick, provide, ref } from 'vue';
import { parseSafeMarkdown } from '@/utils/safeMarkdown';
import type { LinkInteraction } from '@/utils/externalLinks';
import IconButton from '@/components/ui/IconButton.vue';
import SafeMarkdownNode from './SafeMarkdownNode.vue';

const props = withDefaults(defineProps<{
  text: string;
  allowRemoteImages?: boolean;
}>(), {
  allowRemoteImages: false,
});

const emit = defineEmits<{
  'open-link': [payload: LinkInteraction];
  'link-hover': [payload: LinkInteraction | null];
}>();

const document = computed(() => parseSafeMarkdown(props.text));
const previewImage = ref<{ src: string; alt: string } | null>(null);
const lightbox = ref<HTMLElement | null>(null);

provide('satchel-allow-markdown-images', computed(() => props.allowRemoteImages));
provide('satchel-open-markdown-link', (payload: LinkInteraction) => {
  emit('open-link', payload);
});
provide('satchel-hover-markdown-link', (payload: LinkInteraction | null) => {
  emit('link-hover', payload);
});
provide('satchel-open-markdown-image', async (payload: { src: string; alt: string }) => {
  previewImage.value = payload;
  await nextTick();
  lightbox.value?.focus();
});

function closeImagePreview() {
  previewImage.value = null;
}
</script>

<template>
  <div class="safe-markdown">
    <SafeMarkdownNode
      v-for="(block, index) in document.blocks"
      :key="index"
      :node="block"
    />
  </div>
  <Teleport to="body">
    <Transition name="md-lightbox">
      <div
        v-if="previewImage"
        ref="lightbox"
        class="md-lightbox"
        role="dialog"
        aria-modal="true"
        :aria-label="$t('mail.markdown.imagePreview')"
        tabindex="-1"
        @click.self="closeImagePreview"
        @keydown.esc="closeImagePreview"
      >
        <figure class="md-lightbox__figure">
          <img
            class="md-lightbox__image"
            :src="previewImage.src"
            :alt="previewImage.alt"
            referrerpolicy="no-referrer"
          >
          <figcaption v-if="previewImage.alt" class="md-lightbox__caption">
            {{ previewImage.alt }}
          </figcaption>
        </figure>
        <IconButton
          size="lg"
          class="md-lightbox__close"
          :title="$t('mail.markdown.closeImagePreview')"
          @click="closeImagePreview"
        >
          <svg aria-hidden="true" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
            <path d="m5 5 10 10M15 5 5 15" />
          </svg>
        </IconButton>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.safe-markdown {
  min-width: 0;
  max-width: 100%;
  color: inherit;
  font: inherit;
  line-height: 1.65;
  overflow-wrap: anywhere;
}

.safe-markdown :deep(p) {
  margin: 0 0 0.8em;
}

.safe-markdown :deep(p:last-child) {
  margin-bottom: 0;
}

.safe-markdown :deep(h1),
.safe-markdown :deep(h2),
.safe-markdown :deep(h3) {
  margin: 1.1em 0 0.45em;
  color: var(--text-primary);
  font-family: var(--font-sans);
  font-weight: var(--weight-bold);
  letter-spacing: -0.025em;
  line-height: 1.25;
}

.safe-markdown :deep(h1:first-child),
.safe-markdown :deep(h2:first-child),
.safe-markdown :deep(h3:first-child) {
  margin-top: 0;
}

.safe-markdown :deep(h1) { font-size: 1.45em; }
.safe-markdown :deep(h2) { font-size: 1.25em; }
.safe-markdown :deep(h3) { font-size: 1.08em; }

.safe-markdown :deep(ol),
.safe-markdown :deep(ul) {
  margin: 0.3em 0 0.9em;
  padding-left: 1.65em;
}

.safe-markdown :deep(li) {
  padding-left: 0.25em;
  margin: 0.28em 0;
}

.safe-markdown :deep(li > p) {
  margin: 0;
}

.safe-markdown :deep(blockquote) {
  margin: 0.8em 0;
  padding: 0.1em 0 0.1em 1em;
  border-left: 3px solid color-mix(in srgb, var(--accent-primary) 55%, var(--border-color));
  color: var(--text-secondary);
}

.safe-markdown :deep(.md-inline-code) {
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--bg-overlay) 62%, transparent);
  padding: 0.08em 0.34em;
  color: color-mix(in srgb, var(--accent-cyan) 72%, var(--text-primary));
  font-family: var(--font-mono);
  font-size: 0.9em;
}

.safe-markdown :deep(.md-code-block) {
  margin: 0.85em 0;
  overflow: auto;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--bg-inset) 86%, transparent);
  padding: 13px 15px;
  color: var(--text-primary);
  white-space: pre;
  font-family: var(--font-mono);
  font-size: 0.9em;
  line-height: 1.55;
}

.safe-markdown :deep(.md-link) {
  overflow-wrap: anywhere;
  word-break: break-word;
  border-radius: 2px;
  color: color-mix(in srgb, var(--accent-primary) 82%, var(--text-primary));
  text-decoration: underline;
  text-decoration-color: transparent;
  text-underline-offset: 0.18em;
  transition: color var(--duration-fast) var(--ease-out), text-decoration-color var(--duration-fast) var(--ease-out);
}

.safe-markdown :deep(.md-link:hover),
.safe-markdown :deep(.md-link:focus-visible) {
  color: var(--accent-primary);
  text-decoration-color: currentColor;
}

.safe-markdown :deep(.md-link:active) {
  background: color-mix(in srgb, var(--accent-primary) 20%, transparent);
  color: color-mix(in srgb, var(--accent-primary) 72%, white);
}

.safe-markdown :deep(.md-link:focus-visible) {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.safe-markdown :deep(.md-link--risky) {
  text-decoration-color: color-mix(in srgb, var(--accent-amber) 75%, transparent);
  text-decoration-style: wavy;
}

.safe-markdown :deep(.md-link-warning) {
  margin-left: 0.25em;
  color: var(--accent-amber);
  font-size: 0.72em;
  vertical-align: 0.15em;
}

.safe-markdown :deep(.md-blocked-image) {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin: 0.15em 0;
  border: 1px dashed color-mix(in srgb, var(--text-tertiary) 55%, transparent);
  border-radius: var(--radius-xs);
  padding: 0.15em 0.5em;
  color: var(--text-tertiary);
  font-family: var(--font-sans);
  font-size: 0.82em;
}

.safe-markdown :deep(.md-blocked-image--sized) {
  max-height: min(32vh, 240px);
  box-sizing: border-box;
  min-height: 2.2em;
  background: color-mix(in srgb, var(--bg-inset) 58%, transparent);
  text-align: center;
}

.safe-markdown :deep(.md-blocked-image--sized > span) {
  padding: 0.4em;
}

.safe-markdown :deep(.md-image-button) {
  display: block;
  width: 100%;
  max-width: 100%;
  margin: 0.35em 0;
  overflow: hidden;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  padding: 0;
  cursor: zoom-in;
  text-align: left;
}

.safe-markdown :deep(.md-image-button:focus-visible) {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.safe-markdown :deep(.md-image) {
  display: block;
  width: 100%;
  height: auto;
  max-width: 100%;
  max-height: min(58vh, 560px);
  object-fit: contain;
  object-position: left top;
}

.safe-markdown :deep(.md-image-error) {
  display: inline-flex;
  max-width: 100%;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.35em;
  margin: 0.35em 0;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--bg-inset) 68%, transparent);
  padding: 0.38em 0.58em;
  color: var(--text-tertiary);
  font-family: var(--font-sans);
  font-size: 0.82em;
  line-height: 1.4;
}

.safe-markdown :deep(.md-raw-text) {
  white-space: pre-wrap;
}

.safe-markdown :deep(hr) {
  margin: 1em 0;
  border: 0;
  border-top: 1px solid var(--border-color);
}

.safe-markdown :deep(.md-table-scroll) {
  max-width: 100%;
  margin: 0.8em 0;
  overflow-x: auto;
}

.safe-markdown :deep(table) {
  width: 100%;
  border-collapse: collapse;
  font-family: var(--font-sans);
  font-size: 0.9em;
}

.safe-markdown :deep(th),
.safe-markdown :deep(td) {
  border: 1px solid var(--border-color);
  padding: 7px 9px;
}

.safe-markdown :deep(th) {
  background: color-mix(in srgb, var(--bg-overlay) 55%, transparent);
  color: var(--text-primary);
  font-weight: var(--weight-strong);
}

.md-lightbox {
  position: fixed;
  z-index: 1000;
  inset: 0;
  display: grid;
  place-items: center;
  background: var(--lightbox-scrim);
  padding: clamp(16px, 4vw, 48px);
  backdrop-filter: blur(8px);
}

.md-lightbox__figure {
  display: grid;
  max-width: 100%;
  max-height: 100%;
  margin: 0;
  gap: 10px;
  place-items: center;
}

.md-lightbox__image {
  display: block;
  max-width: 100%;
  max-height: calc(100vh - 112px);
  border-radius: var(--radius-sm);
  box-shadow: var(--lightbox-shadow);
  object-fit: contain;
}

.md-lightbox__caption {
  max-width: min(720px, 90vw);
  overflow: hidden;
  color: color-mix(in srgb, var(--lightbox-ink) 78%, transparent);
  font-family: var(--font-sans);
  font-size: var(--font-size-sm);
  line-height: 1.4;
  text-align: center;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* IconButton size="lg" is already 38x38 and already draws the shared ring; the
   scrim is what the colours have to answer to, so only those are overridden —
   with one extra class each, to beat the kit's own rules without !important. */
.md-lightbox .md-lightbox__close {
  position: absolute;
  top: max(16px, env(safe-area-inset-top));
  right: max(16px, env(safe-area-inset-right));
  border: 1px solid color-mix(in srgb, var(--lightbox-ink) 14%, transparent);
  border-radius: var(--radius-full);
  background: var(--lightbox-chrome);
  color: var(--lightbox-ink);
}

.md-lightbox .md-lightbox__close:hover:not(:disabled) {
  background: color-mix(in srgb, var(--lightbox-chrome) 76%, var(--lightbox-ink));
  color: var(--text-on-accent);
}

.md-lightbox__close svg {
  width: 18px;
  height: 18px;
}

.md-lightbox-enter-active,
.md-lightbox-leave-active {
  transition: opacity var(--duration-fast) var(--ease-out);
}

.md-lightbox-enter-from,
.md-lightbox-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .md-lightbox-enter-active,
  .md-lightbox-leave-active {
    transition: none;
  }
}
</style>
