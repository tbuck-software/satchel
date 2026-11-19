<script setup lang="ts">
/**
 * The blocked-images notice's question before a domain allow on mail that
 * did not pass DMARC (see EmailRenderer's `trustDomain`). Asked inline rather
 * than in a dialog: the message-file viewer renders the same notice and has
 * no dialog host.
 *
 * Its own chunk: the question is rare, and its copy stays out of the
 * catalogue the reader prefetches.
 */
import { onMounted, ref } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';

const props = defineProps<{
  domain: string;
  /** Touch-sized buttons instead of text links. */
  phone: boolean;
}>();

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();

const root = ref<HTMLElement | null>(null);

// The clicked link is gone; the safe answer takes the focus, as in ConfirmDialog.
onMounted(() => {
  root.value?.querySelector<HTMLElement>('[data-domain-cancel]')?.focus();
});
</script>

<template>
  <span ref="root" class="domain-confirm">
    <span class="domain-confirm__text">
      {{ $t('mail.imagePolicy.unverifiedDomain', { domain: props.domain }) }}
    </span>
    <span class="domain-confirm__actions">
      <AppButton
        class="image-guard__action image-guard__action--primary"
        :variant="props.phone ? 'soft' : 'link'"
        size="sm"
        @click="emit('confirm')"
      >
        {{ $t('mail.imagePolicy.allowDomainAnyway') }}
      </AppButton>
      <span v-if="!props.phone" class="domain-confirm__sep" aria-hidden="true">·</span>
      <AppButton
        class="image-guard__action"
        data-domain-cancel
        :variant="props.phone ? 'ghost' : 'link'"
        size="sm"
        @click="emit('cancel')"
      >
        {{ $t('common.cancel') }}
      </AppButton>
    </span>
  </span>
</template>

<style scoped>
.domain-confirm {
  display: flex;
  flex: 1 1 auto;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2xs) var(--space-sm);
  min-width: 0;
}

.domain-confirm__text {
  flex: 1 1 24ch;
  color: var(--text-primary);
}

.domain-confirm__actions {
  display: inline-flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2xs) var(--space-sm);
}

.domain-confirm__sep {
  color: var(--text-tertiary);
}
</style>
