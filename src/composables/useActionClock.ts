import { onScopeDispose, ref, watch } from 'vue';
import { useEventListener } from '@vueuse/core';

/** Keep time-based choices current while visible, including after sleep. */
export function useActionClock(active: () => boolean) {
  const now = ref(Date.now());
  const refresh = () => { now.value = Date.now(); };
  let timer: ReturnType<typeof setInterval> | undefined;
  watch(active, (enabled) => {
    clearInterval(timer);
    timer = undefined;
    refresh();
    if (enabled) timer = setInterval(refresh, 1000);
  }, { immediate: true, flush: 'sync' });
  const refreshWhenActive = () => { if (active()) refresh(); };
  useEventListener(window, 'focus', refreshWhenActive);
  useEventListener(document, 'visibilitychange', refreshWhenActive);
  onScopeDispose(() => clearInterval(timer));
  return { now, refresh };
}
