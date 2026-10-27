export interface ListKeyboardNavigationController {
  move: (delta: 1 | -1, extendSelection?: boolean) => boolean;
  open: () => boolean;
  back: () => boolean;
  focusThread?: (threadId: string) => boolean | Promise<boolean>;
  /**
   * The mails the mail keys (`e`, `s`, `r`, …) act on, for a list that shows
   * mail the thread store has not selected: search hits, a topic's feed, a
   * sender under review. Leave it out and the keys act on the store's
   * selection, as in the Inbox.
   */
  targets?: () => string[];
  /**
   * The list checks rows in the thread store's selection, as the list search
   * does over the Inbox: `x`, select-all and Esc work there, and a checked
   * selection outranks `targets`, which then name only the row in focus.
   */
  sharesSelection?: boolean;
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

/** The active list's own targets, or null when the thread store's selection applies. */
export function listKeyboardTargets(): string[] | null {
  return activeController()?.targets?.() ?? null;
}

/** Whether the active list's checked rows live in the thread store's selection. */
export function listKeyboardSharesSelection(): boolean {
  return activeController()?.sharesSelection ?? false;
}

export function leaveListKeyboardLevel(): boolean {
  return activeController()?.back() ?? false;
}

/** Reveal a restored mail through the mounted list, including virtual rows. */
export async function focusListThread(threadId: string): Promise<boolean> {
  return await activeController()?.focusThread?.(threadId) ?? false;
}
