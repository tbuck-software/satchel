<script setup lang="ts">
/**
 * The phone's half of the blocked-images notice: a real Show button and the
 * two "always allow" choices behind one ⋯, each a 44pt target. Inline links
 * there were 24pt tall and wrapped into a line led by a stray "·".
 *
 * Its own chunk, loaded only on the phone: the message-file viewer renders
 * the same notice and stays inside its bundle budget without it.
 */
import { ref } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';
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
  <AppButton
    class="image-guard__show"
    :title="$t('mail.imagePolicy.showOnceTitle')"
    @click="emit('showOnce')"
    variant="soft"
    size="sm"
  >
    {{ $t('mail.imagePolicy.showOnce') }}
  </AppButton>
  <IconButton
    v-if="props.senderEmail || props.domain"
    class="image-guard__choices"
    :title="$t('mail.imagePolicy.choicesLabel')"
    aria-haspopup="dialog"
    :aria-expanded="choicesOpen"
    @click="choicesOpen = true"
  >
    <AppIcon size="lg"><circle cx="3" cy="8" r=".75" fill="currentColor" /><circle cx="8" cy="8" r=".75" fill="currentColor" /><circle cx="13" cy="8" r=".75" fill="currentColor" /></AppIcon>
  </IconButton>
  <ImagePolicyChoices
    :open="choicesOpen"
    :sender-email="props.senderEmail"
    :domain="props.domain"
    @dismiss="choicesOpen = false"
    @allow-sender="emit('allowSender')"
    @allow-domain="emit('allowDomain')"
  />
</template>
