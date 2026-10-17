<script setup lang="ts">
import { computed } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import TopicDetail from '@/components/topics/TopicDetail.vue';

const route = useRoute();
const router = useRouter();

const topicId = computed(() => String(route.params.id ?? ''));

function openThread(threadId: string) {
  void router.push({ name: 'thread', params: { id: threadId } });
}
</script>

<template>
  <TopicDetail
    :topic-id="topicId"
    @deleted="router.push({ name: 'inbox' })"
    @open-thread="openThread"
  />
</template>
