<script setup lang="ts">
import { nextTick, ref } from 'vue';
import EmailRenderer from '@/components/mail/EmailRenderer.vue';
import type { RenderSegment } from '@/utils/emailSegments';
import type { Attachment } from '@/types/mail';

interface RenderFidelityPayload {
  id: string;
  label: string;
  subject: string;
  fromAddress: string;
  fromName?: string;
  date?: string;
  bodyHtml?: string;
  bodyText?: string;
  bodySegments?: RenderSegment[];
  normalizeQuotes?: boolean;
  trustedDmarcPass?: boolean;
  attachments: Attachment[];
  meta: string[];
}

declare global {
  interface Window {
    __setSatchelRenderFidelityPayload?: (payload: RenderFidelityPayload) => Promise<void>;
  }
}

const payload = ref<RenderFidelityPayload | null>(null);

window.__setSatchelRenderFidelityPayload = async (nextPayload: RenderFidelityPayload) => {
  document.documentElement.dataset.renderReady = 'false';
  payload.value = nextPayload;
  await nextTick();
  window.dispatchEvent(new Event('resize'));
  await new Promise((resolve) => window.setTimeout(resolve, 900));
  document.documentElement.dataset.renderReady = 'true';
};
</script>

<template>
  <main class="render-page">
    <section v-if="payload" class="render-shell">
      <header class="render-header">
        <div class="account">{{ payload.label }}</div>
        <h1>{{ payload.subject }}</h1>
        <div class="meta">
          <span>{{ payload.fromName || payload.fromAddress }} &lt;{{ payload.fromAddress }}&gt;</span>
          <span v-if="payload.date">{{ payload.date }}</span>
          <span v-for="entry in payload.meta" :key="entry">{{ entry }}</span>
        </div>
      </header>

      <EmailRenderer
        :html="payload.bodyHtml"
        :text="payload.bodyText"
        :body-segments="payload.bodySegments"
        :attachments="payload.attachments"
        :sender-email="payload.fromAddress"
        :trusted-dmarc-pass="payload.trustedDmarcPass"
        :normalize-quotes="payload.normalizeQuotes ?? false"
      />
    </section>
    <section v-else class="empty">Waiting for render payload...</section>
  </main>
</template>

<style scoped>
.render-page {
  min-height: 100%;
  background: var(--bg-base);
  color: var(--text-primary);
  padding: 28px;
}

.render-shell {
  width: min(1040px, 100%);
  margin: 0 auto;
}

.render-header {
  display: grid;
  gap: 10px;
  margin-bottom: 24px;
}

.account {
  width: fit-content;
  border-radius: var(--radius-xs);
  background: color-mix(in srgb, var(--accent-primary) 18%, transparent);
  color: var(--accent-primary);
  padding: 3px 8px;
  font-size: var(--font-size-sm);
  font-weight: var(--weight-strong);
}

h1 {
  margin: 0;
  color: var(--text-primary);
  font-size: 30px;
  line-height: 1.15;
  letter-spacing: 0;
}

.meta {
  display: grid;
  gap: 3px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.empty {
  color: var(--text-secondary);
}
</style>
