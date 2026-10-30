<script setup lang="ts">
import { computed } from 'vue';
import { useTopicsStore } from '@/stores/topics';
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import type { TopicItemType } from '@/types/topics';
const props = defineProps<{ itemType: TopicItemType; itemIds: string[] }>();
const topics = useTopicsStore();
const failed = computed(() => topics.membershipsFailedFor(props.itemType, props.itemIds));
const loading = computed(() => topics.membershipsLoadingFor(props.itemType, props.itemIds));
const hasContent = computed(() => props.itemIds.every((id) => topics.hasMembershipsFor(props.itemType, id)));
</script>

<template>
  <div class="membership-state" :aria-busy="loading">
    <CollectionLoadState
      :loading="loading" :failed="failed && !loading" :has-content="hasContent"
      :error-label="$t('topics.memberships.loadFailed')"
      @retry="topics.loadMembershipsFor(itemType, itemIds)"
    />
  </div>
</template>

<style scoped>
.membership-state { position: relative; }
</style>
