import type { ComputedRef, InjectionKey } from 'vue';

/**
 * The terms a list search is looking for. The search results provide them and
 * the thread rows inside mark them, without threading a prop through the
 * virtualized list that most callers never search.
 */
export const LIST_HIGHLIGHT_TERMS: InjectionKey<ComputedRef<string[]>> = Symbol('listHighlightTerms');
