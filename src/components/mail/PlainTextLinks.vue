<script setup lang="ts">
import { computed } from 'vue';
import { tokenizePlainTextLinks } from '@/utils/plainTextLinks';
import { assessExternalLink, type LinkInteraction } from '@/utils/externalLinks';

const props = defineProps<{
  text: string;
}>();

const emit = defineEmits<{
  'open-link': [payload: LinkInteraction];
  'link-hover': [payload: LinkInteraction | null];
}>();

const tokens = computed(() => tokenizePlainTextLinks(props.text));

function isRisky(href: string, label: string): boolean {
  return assessExternalLink(href, label).requiresConfirmation;
}

function interaction(event: MouseEvent, href: string, label: string): LinkInteraction {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  return {
    href,
    label,
    anchor: { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom },
  };
}
</script>

<template>
  <pre><template v-for="(token, index) in tokens" :key="index"><a
    v-if="token.kind === 'link'"
    class="plain-text-link"
    :href="token.href"
    :data-risky="isRisky(token.href, token.text) || undefined"
    target="_blank"
    rel="noopener noreferrer"
    @click.prevent="emit('open-link', interaction($event, token.href, token.text))"
    @mouseenter="emit('link-hover', interaction($event, token.href, token.text))"
    @mouseleave="emit('link-hover', null)"
  >{{ token.text }}<span
    v-if="isRisky(token.href, token.text)"
    class="plain-text-link-warning"
    :aria-label="$t('mail.link.warningLabel')"
  >⚠</span></a><template v-else>{{ token.text }}</template></template></pre>
</template>

<style scoped>
.plain-text-link {
  border-radius: 2px;
  color: color-mix(in srgb, var(--accent-primary) 82%, var(--text-primary));
  cursor: pointer;
  text-decoration-line: underline;
  text-decoration-color: transparent;
  text-decoration-thickness: 1px;
  text-underline-offset: 0.18em;
  transition: color var(--duration-fast) var(--ease-out),
    text-decoration-color var(--duration-fast) var(--ease-out);
}

.plain-text-link:hover {
  color: var(--accent-primary);
  text-decoration-color: currentColor;
}

.plain-text-link:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
  text-decoration-color: currentColor;
}

.plain-text-link:active {
  background: color-mix(in srgb, var(--accent-primary) 20%, transparent);
  color: color-mix(in srgb, var(--accent-primary) 72%, white);
}

.plain-text-link[data-risky] {
  text-decoration-color: color-mix(in srgb, var(--accent-amber) 75%, transparent);
  text-decoration-style: wavy;
}

.plain-text-link-warning {
  margin-left: 0.25em;
  color: var(--accent-amber);
  font-family: var(--font-sans);
  font-size: 0.72em;
  vertical-align: 0.15em;
}
</style>
