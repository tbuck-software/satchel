import { router } from '@/router';
import { useLabsStore } from '@/stores/labs';
import type { LabsFlagId } from './flags';
import { startLabsActivation } from './activation';

/** The paths an experiment's activation adds; a link to one of them outlives the switch. */
const LABS_PATHS: Record<LabsFlagId, string> = { chats: '/chats' };

/**
 * Labs at app start, loaded lazily by `bootstrap.ts`: read the flags, then
 * run each experiment's activation while its flag is on. A start on the path
 * of a switched-off experiment (a restored window, an old link) lands in the
 * Inbox. Returns the teardown.
 */
export function startLabs(): () => void {
  const labs = useLabsStore();
  void labs.loadOnce().then(() => {
    const current = router.currentRoute.value;
    const stale = (Object.keys(LABS_PATHS) as LabsFlagId[])
      .some((id) => !labs.isEnabled(id) && current.path === LABS_PATHS[id]);
    if (stale && current.matched.length === 0) void router.replace({ name: 'inbox' });
  });
  return startLabsActivation((id) => labs.isEnabled(id));
}
