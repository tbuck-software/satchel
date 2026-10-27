<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import TopicDetail from '@/components/topics/TopicDetail.vue';
import { registerListKeyboardNavigation } from '@/composables/useListKeyboardNavigation';

const route = useRoute();
const router = useRouter();

const topicId = computed(() => String(route.params.id ?? ''));

function openThread(threadId: string) {
  void router.push({ name: 'thread', params: { id: threadId } });
}

// ── Keyboard: j/k step through the topic's mails, the mail keys act on one ──
const detail = ref<InstanceType<typeof TopicDetail> | null>(null);
const focusedThreadId = ref('');

/** The feed's mail cards in order; notes carry no thread and are skipped. */
function mailCards(): HTMLElement[] {
  const root = detail.value?.$el as HTMLElement | undefined;
  return root ? [...root.querySelectorAll<HTMLElement>('.feed [data-thread-id]')] : [];
}

function move(delta: 1 | -1): boolean {
  const cards = mailCards();
  if (cards.length === 0) return false;
  const index = cards.findIndex((card) => card === document.activeElement);
  const card = cards[index < 0 ? 0 : Math.min(cards.length - 1, Math.max(0, index + delta))]!;
  card.tabIndex = -1;
  card.focus();
  card.scrollIntoView({ block: 'nearest' });
  focusedThreadId.value = card.dataset.threadId ?? '';
  return true;
}

onBeforeUnmount(registerListKeyboardNavigation({
  move,
  open: () => {
    if (!focusedThreadId.value) return false;
    openThread(focusedThreadId.value);
    return true;
  },
  back: () => false,
  targets: () => (focusedThreadId.value ? [focusedThreadId.value] : []),
}));
</script>

<template>
  <TopicDetail
    ref="detail"
    :topic-id="topicId"
    @deleted="router.push({ name: 'inbox' })"
    @open-thread="openThread"
  />
</template>
