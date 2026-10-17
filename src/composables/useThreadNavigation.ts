import { useThreadsStore } from '@/stores/threads';

export function useThreadNavigation() {
  const threadsStore = useThreadsStore();

  return {
    next: threadsStore.nextThread,
    prev: threadsStore.prevThread,
    select: threadsStore.selectThread,
  };
}
