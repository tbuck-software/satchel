export interface ThreadMessageKeyboardNavigationController {
  move: (delta: 1 | -1) => boolean;
  open: () => boolean;
  toggleFocused: () => boolean;
  toggleAll: () => boolean;
}

let activeController: ThreadMessageKeyboardNavigationController | null = null;

/**
 * Bridges the global shortcut handler to the currently mounted reader. The
 * handler only calls this bridge on the dedicated thread route, so the reader
 * preview beside the inbox never steals J/K from the list.
 */
export function registerThreadMessageKeyboardNavigation(
  controller: ThreadMessageKeyboardNavigationController,
): () => void {
  activeController = controller;
  return () => {
    if (activeController === controller) activeController = null;
  };
}

export function moveThreadMessageFocus(delta: 1 | -1): boolean {
  return activeController?.move(delta) ?? false;
}

export function openThreadMessageFocus(): boolean {
  return activeController?.open() ?? false;
}

export function toggleFocusedThreadMessage(): boolean {
  return activeController?.toggleFocused() ?? false;
}

export function toggleAllThreadMessages(): boolean {
  return activeController?.toggleAll() ?? false;
}
