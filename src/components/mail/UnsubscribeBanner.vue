<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
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
  <!-- An offer, not a warning: a quiet row in the reader's own greys. The
       amber and red notices beside it mean "be careful"; this one does not. -->
  <aside v-if="!dismissed" class="unsub">
    <AppIcon class="unsub__icon"><rect x="2" y="3.5" width="12" height="9" rx="1.5" /><path d="m2.5 4.5 5.5 4 5.5-4" /></AppIcon>
    <span class="unsub__text">{{ $t('mail.unsubscribe.banner') }}</span>
    <div class="unsub__actions">
      <AppButton :disabled="busy" @click="unsubscribe" variant="secondary" size="sm">
        {{ busy ? $t('mail.unsubscribe.sending') : $t('mail.unsubscribe.action') }}
      </AppButton>
      <AppButton @click="dismissed = true" variant="ghost" size="sm">{{ $t('common.close') }}</AppButton>
    </div>
  </aside>
</template>

<style scoped>
.unsub {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--space-xs) var(--space-sm);
  margin: var(--space-sm) 0;
  padding: var(--space-1-5) var(--space-1-5) var(--space-1-5) var(--space-md);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.unsub__icon {
  flex-shrink: 0;
  color: var(--text-tertiary);
}

.unsub__text {
  /* Below about this width the buttons take a line of their own rather than
     squeezing the sentence into a narrow column. */
  flex: 1 1 180px;
  min-width: 0;
}

.unsub__actions {
  display: flex;
  gap: var(--space-xs);
  margin-left: auto;
}
</style>
