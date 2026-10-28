<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import { computed, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useMessagesStore } from '@/stores/messages';
import { useToast } from '@/composables/useToast';
import { translateBackendError } from '@/utils/backendErrors';

/**
 * Collapsible view of the conversation being answered, at the bottom of the
 * composer. The editable outgoing quote is prepared separately by the composer.
 */
const props = defineProps<{
  threadId: string;
}>();

const { locale, t } = useI18n();
const messagesStore = useMessagesStore();
const toast = useToast();

const expanded = ref(false);
const loading = ref(false);

const messages = computed(() => messagesStore.getByThread(props.threadId));

// The conversation's messages, not the unsent draft lying at its end.
const sentCount = computed(() => messages.value.filter((message) => !message.isDraft).length);

const toggleLabel = computed(() =>
  expanded.value
    ? t('compose.context.hide')
    : t('compose.context.show', sentCount.value),
);

async function toggle() {
  expanded.value = !expanded.value;
  if (!expanded.value || messages.value.length > 0) return;
  loading.value = true;
  try {
    await messagesStore.loadByThread(props.threadId);
  } catch (error) {
    console.warn('thread context load failed', error);
    toast.error(t('compose.context.loadFailed', {
      error: translateBackendError(error) ?? (error instanceof Error ? error.message : String(error)),
    }));
  } finally {
    loading.value = false;
  }
}

function formatDate(value: string) {
  try {
    return new Date(value).toLocaleString(locale.value, {
      dateStyle: 'medium',
      timeStyle: 'short',
    });
  } catch {
    return value;
  }
}

function senderLabel(message: (typeof messages.value)[number]) {
  return message.from.name || message.from.email;
}
</script>

<template>
  <div class="thread-context" data-composer-context>
    <AppButton
      class="context-toggle"
      :aria-expanded="expanded"
      @click="toggle"
      variant="link"
      size="sm"
    >
      <svg
        :class="['context-chevron', { 'context-chevron--open': expanded }]"
        width="11"
        height="11"
        viewBox="0 0 12 12"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      ><path d="m4.5 3.5 2.5 2.5-2.5 2.5" /></svg>
      {{ toggleLabel }}
    </AppButton>

    <div v-if="expanded" class="context-list">
      <p v-if="loading" class="context-empty">{{ $t('compose.context.loading') }}</p>
      <p v-else-if="messages.length === 0" class="context-empty">{{ $t('compose.context.empty') }}</p>
      <article v-for="message in messages" :key="message.id" class="context-message">
        <header class="context-message-head">
          <strong class="context-sender">{{ senderLabel(message) }}</strong>
          <time class="context-date">{{ formatDate(message.timestamp) }}</time>
        </header>
        <pre class="context-body">{{ message.bodyText?.trim() || $t('compose.context.noTextContent') }}</pre>
      </article>
    </div>
  </div>
</template>

<style scoped>
.thread-context {
  flex: none;
  margin: 10px 2px 2px;
  padding-top: 8px;
  border-top: 1px dashed var(--border-color-muted);
  cursor: default;
}

.context-chevron {
  flex: none;
  transition: transform var(--duration-normal) var(--ease-out);
}

.context-chevron--open {
  transform: rotate(90deg);
}

.context-list {
  display: grid;
  gap: 10px;
  max-height: 260px;
  overflow: auto;
  margin-top: 8px;
  padding: 10px 12px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-base) 40%, transparent);
}

.context-empty {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.context-message {
  display: grid;
  gap: 4px;
}

.context-message + .context-message {
  padding-top: 10px;
  border-top: 1px solid var(--border-color-muted);
}

.context-message-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}

.context-sender {
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}

.context-date {
  flex: none;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.context-body {
  margin: 0;
  max-height: 130px;
  overflow: auto;
  color: var(--text-tertiary);
  font: inherit;
  font-size: var(--font-size-sm);
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
