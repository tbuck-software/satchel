<script setup lang="ts">
import { onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { useThreadsStore } from '@/stores/threads';

const router = useRouter();
const threads = useThreadsStore();

onMounted(() => {
  threads.workflowFilter = 'waiting';
  threads.setMailboxFilter({ kind: 'all', value: null });
  void router.replace({ name: 'inbox' });
});
</script>

<template>
  <div />
</template>
