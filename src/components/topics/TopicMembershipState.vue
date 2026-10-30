<script setup lang="ts">
import { computed } from 'vue';
import { useTopicsStore } from '@/stores/topics';
import CollectionLoadState from '@/components/ui/CollectionLoadState.vue';
import type { TopicItemType } from '@/types/topics';
const props = defineProps<{ itemType: TopicItemType; itemIds: string[] }>();
const topics = useTopicsStore();
const failed = computed(() => topics.membershipsFailedFor(props.itemType, props.itemIds));
const known = computed(() => topics.membershipsKnownFor(props.itemType, props.itemIds));
</script>

<template>
  <div class="membership-state" :aria-busy="!known && !failed">
    <CollectionLoadState
      :loading="!known && !failed" :failed="failed" :has-content="false"
      :error-label="$t('topics.memberships.loadFailed')"
      @retry="topics.loadMembershipsFor(itemType, itemIds)"
    />
  </div>
</template>

<style scoped>
.membership-state { position: relative; }
</style>
