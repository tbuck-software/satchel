export interface ListKeyboardNavigationController {
  move: (delta: 1 | -1, extendSelection?: boolean) => boolean;
  open: () => boolean;
  back: () => boolean;
  focusThread?: (threadId: string) => boolean | Promise<boolean>;
}

/*
 * A stack, not a slot: the search results register over the Inbox list and
 * must hand the keyboard back to it when they close, without the Inbox having
 * to register again.
 */
const controllers: ListKeyboardNavigationController[] = [];

function activeController(): ListKeyboardNavigationController | undefined {
  return controllers.at(-1);
}

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
  controllers.push(controller);
  return () => {
    const index = controllers.lastIndexOf(controller);
    if (index >= 0) controllers.splice(index, 1);
  };
}

export function moveListKeyboardFocus(delta: 1 | -1, extendSelection = false): boolean {
  return activeController()?.move(delta, extendSelection) ?? false;
}

export function openListKeyboardFocus(): boolean {
  return activeController()?.open() ?? false;
}

export function leaveListKeyboardLevel(): boolean {
  return activeController()?.back() ?? false;
}

/** Reveal a restored mail through the mounted list, including virtual rows. */
export async function focusListThread(threadId: string): Promise<boolean> {
  return await activeController()?.focusThread?.(threadId) ?? false;
}
