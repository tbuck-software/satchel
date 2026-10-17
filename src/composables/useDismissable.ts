import { onBeforeUnmount } from 'vue';
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
