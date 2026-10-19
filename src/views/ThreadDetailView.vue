<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useThreadsStore } from '@/stores/threads';
import ThreadView from '@/components/mail/ThreadView.vue';
import ThreadPaneSkeleton from '@/components/mail/ThreadPaneSkeleton.vue';

const route = useRoute();
const router = useRouter();
const threads = useThreadsStore();

const threadId = computed(() => String(route.params.id ?? ''));
const loadingThread = ref(false);
const hasThread = computed(() => threads.threads.some((thread) => thread.id === threadId.value));
let loadRevision = 0;

watch(
  threadId,
  async (next, previous) => {
    const revision = ++loadRevision;
    if (previous && previous !== next) {
      threads.releaseRetainedThread(previous);
    }
    if (!next) return;

    threads.selectThread(next);
    loadingThread.value = !threads.threads.some((thread) => thread.id === next);
    await threads.ensureThreadLoaded(next, { retainAcrossListLoads: true });
    if (revision === loadRevision) {
      loadingThread.value = false;
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  threads.releaseRetainedThread(threadId.value);
});

function goBack() {
  if (window.history.length > 1) {
    router.back();
  } else {
    void router.push({ name: 'inbox' });
  }
}
</script>

<template>
  <section class="thread-page">
    <ThreadView v-if="hasThread" :thread-id="threadId" show-back @back="goBack" />
    <ThreadPaneSkeleton v-else-if="loadingThread" />
    <div v-else class="thread-state">
      <span>{{ $t('thread.detail.unavailable') }}</span>
      <AppButton class="thread-back" @click="goBack" variant="link" size="sm">{{ $t('thread.detail.backToInbox') }}</AppButton>
    </div>
  </section>
</template>

<style scoped>
.thread-page {
  height: 100%;
  background: var(--bg-surface);
  overflow: hidden;
}

.thread-state {
  display: flex;
  height: 100%;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 14px;
  padding: 24px;
  color: var(--text-tertiary);
  text-align: center;
}

</style>
