<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue';
import AppButton from './AppButton.vue';
import SkeletonRow from './SkeletonRow.vue';
const props = defineProps<{ loading: boolean; failed: boolean; hasContent: boolean; errorLabel: string }>();
defineEmits<{ retry: [] }>();
const delayed = ref(false);
let timer: ReturnType<typeof setTimeout> | undefined;
watch(() => props.loading, (loading) => {
  clearTimeout(timer);
  delayed.value = false;
  if (loading) timer = setTimeout(() => { delayed.value = true; }, 160);
}, { immediate: true });
onBeforeUnmount(() => clearTimeout(timer));
</script>

<template>
  <div class="collection-load-state">
    <div v-if="failed" class="collection-error" role="alert">
      <p class="state state--error">{{ errorLabel }}</p>
      <AppButton size="sm" @click="$emit('retry')">{{ $t('common.retry') }}</AppButton>
    </div>
    <div v-else-if="loading && delayed && hasContent" class="collection-refresh" role="status">
      <span class="sr-only">{{ $t('common.loading') }}</span>
    </div>
    <div v-else-if="loading && delayed && !hasContent" role="status">
      <span class="sr-only">{{ $t('common.loading') }}</span>
      <SkeletonRow :count="3" aria-hidden="true" />
    </div>
  </div>
</template>

<style scoped>
.collection-load-state { position: relative; }
.collection-refresh {
  position: absolute;
  inset: 0 0 auto;
  height: var(--space-2xs);
  background: var(--accent-primary);
  pointer-events: none;
  z-index: 1;
}
.collection-error { display: grid; gap: var(--space-sm); padding: var(--space-sm); }
.state { margin: 0; font-size: var(--font-size-sm); }
.state--error { color: var(--accent-red-soft); }
</style>
