<script setup lang="ts">
/**
 * The phone's half of the blocked-images notice: one split button, "Show"
 * and an attached chevron for the two "always allow" choices, each a 44pt
 * target. Inline links there were 24pt tall and wrapped into a line led by a
 * stray "·"; a separate ⋯ sat right under the message's own ⋯ and read as a
 * second menu for the same thing.
 *
 * Its own chunk, loaded only on the phone: the message-file viewer renders
 * the same notice and stays inside its bundle budget without it.
 */
import { ref } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import ImagePolicyChoices from './ImagePolicyChoices.vue';

const props = defineProps<{
  senderEmail: string;
  /** Empty when a domain-wide allow is not offered. */
  domain: string;
}>();

const emit = defineEmits<{
  showOnce: [];
  allowSender: [];
  allowDomain: [];
}>();

const choicesOpen = ref(false);
</script>

<template>
  <span class="image-guard__split">
    <AppButton
      class="image-guard__show"
      :class="{ 'image-guard__show--joined': props.senderEmail || props.domain }"
      :title="$t('mail.imagePolicy.showOnceTitle')"
      @click="emit('showOnce')"
      variant="soft"
      size="sm"
    >
      {{ $t('mail.imagePolicy.showOnce') }}
    </AppButton>
    <AppButton
      v-if="props.senderEmail || props.domain"
      class="image-guard__choices"
      :title="$t('mail.imagePolicy.choicesLabel')"
      :aria-label="$t('mail.imagePolicy.choicesLabel')"
      aria-haspopup="dialog"
      :aria-expanded="choicesOpen"
      variant="soft"
      size="sm"
      @click="choicesOpen = true"
    >
      <AppIcon><path d="m4.5 6.5 3.5 3.5 3.5-3.5" /></AppIcon>
    </AppButton>
  </span>
  <ImagePolicyChoices
    :open="choicesOpen"
    :sender-email="props.senderEmail"
    :domain="props.domain"
    @dismiss="choicesOpen = false"
    @allow-sender="emit('allowSender')"
    @allow-domain="emit('allowDomain')"
  />
</template>

<style scoped>
.image-guard__split {
  display: inline-flex;
  flex: none;
  margin-inline-start: auto;
}

.image-guard__show--joined {
  border-start-end-radius: 0;
  border-end-end-radius: 0;
}

.image-guard__choices {
  padding-inline: var(--space-sm);
  border-start-start-radius: 0;
  border-end-start-radius: 0;
  border-inline-start: 1px solid var(--border-color);
}
</style>
