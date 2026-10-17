<script setup lang="ts">
import { ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useToast } from '@/composables/useToast';
import { useMessagesStore } from '@/stores/messages';
import { backendErrorLabel } from '@/utils/backendErrorLabels';

const { t } = useI18n();

const props = defineProps<{
  messageId: string;
}>();

const dismissed = ref(false);
const busy = ref(false);
const toast = useToast();
const messages = useMessagesStore();

async function unsubscribe() {
  busy.value = true;
  try {
    const result = await messages.unsubscribe(props.messageId);
    toast.success(t('mail.unsubscribe.success', { method: result.method }));
    dismissed.value = true;
  } catch (err) {
    // The backend refuses a target that is not publicly reachable and says so
    // with its own key; the raw message would arrive as the serialized error.
    toast.error(t('mail.unsubscribe.failed', { error: backendErrorLabel(err) }));
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <aside v-if="!dismissed" class="unsub">
    <span>{{ $t('mail.unsubscribe.banner') }}</span>
    <div class="actions">
      <button type="button" class="primary" :disabled="busy" @click="unsubscribe">
        {{ busy ? $t('mail.unsubscribe.sending') : $t('mail.unsubscribe.action') }}
      </button>
      <button type="button" class="ghost" @click="dismissed = true">{{ $t('common.close') }}</button>
    </div>
  </aside>
</template>

<style scoped>
.unsub {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  padding: 8px 12px;
  margin: 8px 0;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-yellow) 8%, var(--bg-raised));
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
.actions { display: flex; gap: 6px; }
.primary {
  background: var(--accent-primary); color: var(--text-on-accent); border: 0;
  padding: 4px 12px; border-radius: var(--radius-sm); cursor: pointer;
  font-size: var(--font-size-xs); font-weight: var(--weight-semibold);
}
.ghost {
  background: transparent; border: 1px solid var(--border-color);
  color: var(--text-secondary); padding: 4px 12px; border-radius: var(--radius-sm);
  cursor: pointer; font-size: var(--font-size-xs);
}
</style>
