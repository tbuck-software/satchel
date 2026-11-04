<script setup lang="ts">
/**
 * The phone's half of the blocked-images notice. In a message it is Show
 * alone: the two "always allow" choices join the message's actions (its ⋯,
 * or the thread's nav-bar ⋯ for the newest message), reported through
 * IMAGE_GUARD_CHOICES. Where no message provides that (the topic feed), they
 * keep a chevron joined to Show, each a 44pt row in a sheet.
 *
 * Its own chunk, loaded only on the phone: the message-file viewer renders
 * the same notice and stays inside its bundle budget without it.
 */
import { computed, defineAsyncComponent, inject, onBeforeUnmount, ref, watchEffect } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { IMAGE_GUARD_CHOICES } from './threadMessageMenu';

const ImagePolicyChoices = defineAsyncComponent(() => import('./ImagePolicyChoices.vue'));

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

const reported = inject(IMAGE_GUARD_CHOICES, null);
const ownChoices = computed(() => !reported && Boolean(props.senderEmail || props.domain));
const choicesOpen = ref(false);

if (reported) {
  watchEffect(() => {
    reported.value = props.senderEmail || props.domain
      ? {
        senderEmail: props.senderEmail,
        domain: props.domain,
        allowSender: () => emit('allowSender'),
        allowDomain: () => emit('allowDomain'),
      }
      : null;
  });
  onBeforeUnmount(() => { reported.value = null; });
}
</script>

<template>
  <span class="image-guard__split">
    <AppButton
      class="image-guard__show"
      :class="{ 'image-guard__show--joined': ownChoices }"
      :title="$t('mail.imagePolicy.showOnceTitle')"
      @click="emit('showOnce')"
      variant="soft"
      size="sm"
    >
      {{ $t('mail.imagePolicy.showOnce') }}
    </AppButton>
    <AppButton
      v-if="ownChoices"
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
    v-if="ownChoices"
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
