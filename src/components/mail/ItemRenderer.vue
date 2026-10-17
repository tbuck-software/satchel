<script setup lang="ts">
import type { Message } from "@/types/mail";
import { useLinkOpener } from "@/composables/useLinkOpener";
import ExternalLinkDialog from "./ExternalLinkDialog.vue";
import SafeMarkdownBody from "./SafeMarkdownBody.vue";

const props = withDefaults(
  defineProps<{ message: Message; showBody?: boolean }>(),
  {
    showBody: true,
  },
);

const linkOpener = useLinkOpener();

function openSource(event: MouseEvent) {
  const url = props.message.sourcePayload?.url;
  if (!url) return;
  event.preventDefault();
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
  linkOpener.open({
    href: url,
    label: "",
    anchor: { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom },
  });
}
</script>

<template>
  <section class="source-item">
    <div class="meta">
      <span class="kind">{{ message.sourcePayload?.kind }}</span>
      <span :class="['level', message.sourcePayload?.level]">
        {{ message.sourcePayload?.level || "info" }}
      </span>
    </div>
    <SafeMarkdownBody
      v-if="showBody"
      class="body"
      :text="message.bodyText"
      @open-link="linkOpener.open"
    />
    <a
      v-if="message.sourcePayload?.url"
      :href="message.sourcePayload.url"
      target="_blank"
      rel="noopener noreferrer"
      @click="openSource"
    >
      {{ $t('mail.item.openInBrowser') }}
    </a>
    <ExternalLinkDialog
      :assessment="linkOpener.pending.value"
      @cancel="linkOpener.cancel"
      @confirm="linkOpener.confirm"
    />
    <details v-if="message.sourcePayload?.data">
      <summary>{{ $t('mail.item.sourceData') }}</summary>
      <pre>{{ JSON.stringify(message.sourcePayload.data, null, 2) }}</pre>
    </details>
  </section>
</template>

<style scoped>
.source-item {
  display: grid;
  gap: 12px;
  margin-top: 18px;
  padding: 18px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: linear-gradient(135deg, var(--bg-raised), var(--bg-surface));
}
.meta {
  display: flex;
  gap: 7px;
}
.kind,
.level {
  padding: 3px 8px;
  border-radius: var(--radius-full);
  font: 700 var(--font-size-2xs)/1 var(--font-mono);
  text-transform: uppercase;
}
.kind {
  color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
}
.level {
  color: var(--text-tertiary);
  background: var(--bg-overlay);
}
.level.warn {
  color: var(--accent-amber);
}
.level.error {
  color: var(--accent-red);
}
.body {
  white-space: pre-wrap;
  line-height: 1.65;
  color: var(--text-secondary);
}
a {
  color: var(--accent-primary);
  text-decoration: none;
}
pre {
  overflow: auto;
  max-height: 260px;
  padding: 12px;
  border-radius: var(--radius-sm);
  background: var(--bg-inset);
  font-size: var(--font-size-2xs);
}
</style>
