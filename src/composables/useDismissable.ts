import { onBeforeUnmount, watch } from 'vue';
import { useUiStore } from '@/stores/ui';

/**
 * Put a surface on the back stack for as long as it is on screen.
 *
 * Every overlay used to answer the escape key and a click on its backdrop, and
 * neither of those exists on a phone. The system back gesture went straight
 * past them to whatever was underneath — or out of the app — leaving a dialog
 * the user could open but not leave.
 *
 * `isOpen` must read reactive state: the store derives its depth from a
 * computed, so a captured boolean would be read once and never again. For
 * overlays the parent mounts only while they are open, `() => true` is the
 * honest answer.
 */
export function useDismissable(isOpen: () => boolean, dismiss: () => void) {
  const ui = useUiStore();
  onBeforeUnmount(ui.registerOverlay({ isOpen, dismiss }));
}

/**
 * `useDismissable` for a surface mounted once for the whole app that opens on
 * top of whatever is up at the time — the confirm and prompt dialogs.
 *
 * Registering at mount would file it beneath every overlay opened later, and
 * Escape or Back would close the sheet behind the question instead of the
 * question. It joins the stack each time it opens, so it is always on top.
 */
export function useDismissableOnTop(isOpen: () => boolean, dismiss: () => void) {
  const ui = useUiStore();
  let unregister: (() => void) | null = null;
  watch(
    isOpen,
    (open) => {
      unregister?.();
      unregister = open ? ui.registerOverlay({ isOpen, dismiss }) : null;
    },
    { immediate: true, flush: 'sync' },
  );
  onBeforeUnmount(() => unregister?.());
}
