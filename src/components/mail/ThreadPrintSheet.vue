<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import type { Message } from '@/types/mail';

const props = defineProps<{
  active: boolean;
  subject: string;
  messages: Message[];
  formatDate: (value: string) => string;
  recipientLine: (message: Message) => string;
}>();

const { t } = useI18n();
</script>

<template>
  <Teleport to="body">
    <section
      v-if="props.active"
      class="thread-print-sheet"
      data-thread-print
      aria-hidden="true"
    >
      <h1>{{ props.subject }}</h1>
      <article v-for="message in props.messages" :key="message.id" class="thread-print-message">
        <header>
          <div>
            <strong>{{ message.from.name }}</strong>
            &lt;{{ message.from.email }}&gt;
          </div>
          <div>{{ props.formatDate(message.timestamp) }}</div>
          <div>{{ t('common.to') }}: {{ props.recipientLine(message) }}</div>
        </header>
        <pre>{{ message.bodyText ?? '' }}</pre>
      </article>
    </section>
  </Teleport>
</template>

<style>
.thread-print-sheet {
  display: none;
}

/* Paper is white whatever the app theme is: these are ink values, not tokens. */
@media print {
  body.satchel-thread-printing > #app {
    display: none !important;
  }

  body.satchel-thread-printing > .thread-print-sheet {
    display: block !important;
    box-sizing: border-box;
    width: 100%;
    max-width: 760px;
    margin: 0 auto;
    padding: 0 16px;
    color: #111;
    background: #fff;
    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
  }

  .thread-print-sheet h1 {
    margin: 0 0 16px;
    padding-bottom: 6px;
    border-bottom: 2px solid #000;
    font-size: 20px;
  }

  .thread-print-message {
    margin: 24px 0;
    padding-bottom: 24px;
    border-bottom: 1px solid #ccc;
  }

  .thread-print-message:last-child {
    border-bottom: 0;
  }

  .thread-print-message header {
    margin-bottom: 12px;
    break-inside: avoid;
    color: #555;
    font-size: var(--font-size-xs);
  }

  .thread-print-message header > div {
    margin: 2px 0;
  }

  .thread-print-message header strong {
    color: #000;
  }

  .thread-print-message pre {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: inherit;
    font-size: var(--font-size-md);
    line-height: 1.55;
  }
}
</style>
