<script setup lang="ts">
/**
 * The "always allow" half of the blocked-images notice on the phone. Inline
 * there, the two choices were 24pt text links that wrapped into a line
 * starting with a stray separator; behind one button they are full rows.
 */
import BottomSheet from '@/components/ui/BottomSheet.vue';
import MenuItem from '@/components/ui/MenuItem.vue';

const props = defineProps<{
  open: boolean;
  senderEmail: string;
  /** Empty when a domain-wide allow is not offered. */
  domain: string;
}>();

const emit = defineEmits<{
  dismiss: [];
  allowSender: [];
  allowDomain: [];
}>();

function choose(action: 'allowSender' | 'allowDomain') {
  emit('dismiss');
  if (action === 'allowSender') emit('allowSender');
  else emit('allowDomain');
}
</script>

<template>
  <Teleport to="body">
    <BottomSheet
      :label="$t('mail.imagePolicy.choicesLabel')"
      :open="props.open"
      @dismiss="emit('dismiss')"
    >
      <nav role="menu" class="image-choices" :aria-label="$t('mail.imagePolicy.choicesLabel')">
        <h2>{{ $t('mail.imagePolicy.choicesLabel') }}</h2>
        <MenuItem
          v-if="props.senderEmail"
          :label="$t('mail.imagePolicy.allowSender')"
          icon="person"
          @select="choose('allowSender')"
        >
          <span class="image-choices__label">
            {{ $t('mail.imagePolicy.allowSender') }}
            <small>{{ props.senderEmail }}</small>
          </span>
        </MenuItem>
        <MenuItem
          v-if="props.domain"
          :label="$t('mail.imagePolicy.allowDomain', { domain: props.domain })"
          icon="link"
          @select="choose('allowDomain')"
        >
          <span class="image-choices__label">
            {{ $t('mail.imagePolicy.allowDomain', { domain: props.domain }) }}
            <small>{{ $t('mail.imagePolicy.allowDomainTitle', { domain: props.domain }) }}</small>
          </span>
        </MenuItem>
      </nav>
    </BottomSheet>
  </Teleport>
</template>

<style scoped>
.image-choices {
  display: grid;
  width: min(420px, 100vw);
  padding: var(--space-sm) var(--space-md) calc(var(--space-md) + var(--satchel-safe-area-bottom));
}

.image-choices h2 {
  margin: 0;
  padding: var(--space-2-5) var(--space-sm) var(--space-md);
  color: var(--text-primary);
  font-size: var(--font-size-lg);
}

.image-choices__label {
  display: grid;
  min-width: 0;
  gap: var(--space-2xs);
}

.image-choices__label small {
  overflow: hidden;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
