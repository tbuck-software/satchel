/**
 * Who answers "back to the top" for the mail list: tapping the Mail tab while
 * already on it, as on iOS. Its own module, like `listSearchFocus`, so the tab
 * bar does not pull the list into its import graph.
 */
let scrollToTop: (() => void) | null = null;

/** False when no mail list is on screen. */
export function scrollListToTop(): boolean {
  if (!scrollToTop) return false;
  scrollToTop();
  return true;
}

export function registerListScrollTop(scroll: () => void): () => void {
  scrollToTop = scroll;
  return () => {
    if (scrollToTop === scroll) scrollToTop = null;
  };
}
