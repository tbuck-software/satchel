export interface ListKeyboardNavigationController {
  move: (delta: 1 | -1, extendSelection?: boolean) => boolean;
  open: () => boolean;
  back: () => boolean;
}

let activeController: ListKeyboardNavigationController | null = null;

/**
 * Connects the global keyboard handler to whichever list workspace is mounted.
 *
 * The Inbox owns knowledge about mixed rows (folders, bundles and mails), while
 * the global handler owns the shortcuts. Keeping the bridge this small avoids
 * duplicating that list model in a store or teaching the keyboard handler about
 * view-local surfaces such as the topics folder.
 */
export function registerListKeyboardNavigation(
  controller: ListKeyboardNavigationController,
): () => void {
  activeController = controller;
  return () => {
    if (activeController === controller) activeController = null;
  };
}

export function moveListKeyboardFocus(delta: 1 | -1, extendSelection = false): boolean {
  return activeController?.move(delta, extendSelection) ?? false;
}

export function openListKeyboardFocus(): boolean {
  return activeController?.open() ?? false;
}

export function leaveListKeyboardLevel(): boolean {
  return activeController?.back() ?? false;
}
