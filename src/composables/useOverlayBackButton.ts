import { onBeforeUnmount, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useUiStore } from '@/stores/ui';

/** Marks the history entries this composable owns, so it ignores real ones. */
const OVERLAY_STATE = 'satchel:overlay';

/**
 * Give the overlays history entries so the back gesture can step through them.
 *
 * Settings, its detail pane, account setup, the omni panel, the shortcut sheet
 * and the mobile sidebar are component state, not routes. Tauri's Android
 * activity decides what back does by asking `WebView.canGoBack()`, so with no
 * entry to return to it left the app — from a dialog the user only wanted to
 * dismiss.
 *
 * One entry per open level, mirrored from `ui.overlayDepth`. Nesting therefore
 * costs one press per level rather than collapsing into a single one, and the
 * store stays the single place that decides what "topmost" means.
 *
 * Nothing native is involved, so a narrow browser window and a narrow app
 * window behave exactly like the phone.
 */
export function useOverlayBackButton() {
  const ui = useUiStore();
  if (typeof window === 'undefined') return;
  const router = useRouter();

  /** Entries we pushed and have not yet consumed. */
  let owned = 0;
  let currentEntryOwned = window.history.state?.satchel === OVERLAY_STATE;

  const onPopState = (event: PopStateEvent) => {
    const leftOwnedEntry = currentEntryOwned;
    currentEntryOwned = event.state?.satchel === OVERLAY_STATE;
    if (!leftOwnedEntry || owned === 0) return;
    owned -= 1;
    // May find nothing left to close — see the note on spent entries below.
    ui.dismissTopmostOverlay();
  };

  window.addEventListener('popstate', onPopState);
  const removeAfterEach = router.afterEach(() => {
    // Router navigations create their own entry. Keep the source overlay entry
    // in the stack, but do not mistake the route entry for one we own when the
    // user later navigates Back through ordinary pages. Vue Router's popstate
    // listener runs before ours, so defer this refresh until every listener has
    // had a chance to inspect the entry that was just left.
    window.setTimeout(() => {
      currentEntryOwned = window.history.state?.satchel === OVERLAY_STATE;
    }, 0);
  });

  const stop = watch(
    () => ui.overlayDepth,
    (depth) => {
      if (depth > owned) {
        for (let i = owned; i < depth; i += 1) {
          // Vue Router stores its current/back/forward bookkeeping in
          // history.state. Dropping it here eventually makes a later
          // router.push() construct an invalid `undefined/` URL.
          window.history.pushState(
            { ...window.history.state, satchel: OVERLAY_STATE },
            '',
          );
        }
        owned = depth;
        currentEntryOwned = true;
        return;
      }
      // Closing from the UI deliberately leaves the entry behind.
      //
      // Reclaiming it with history.go() looked tidier and was wrong: an overlay
      // often closes *because* it navigated — picking a result in the omni
      // panel closes the panel and pushes /search in the same tick — and the
      // rewind then undid the navigation the user had just asked for. Checking
      // the route first does not help, because the watcher runs before
      // router.push settles.
      //
      // The cost is one inert back press after dismissing a surface by hand.
      // The alternative cost was losing the destination, which is worse.
    },
  );

  onBeforeUnmount(() => {
    stop();
    removeAfterEach();
    window.removeEventListener('popstate', onPopState);
  });
}
