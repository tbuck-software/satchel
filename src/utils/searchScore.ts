/**
 * Token matching shared by the command palette and the settings filter.
 *
 * Every typed word has to appear somewhere, in any order: "sig edit" finds
 * "Edit signatures". The rank only orders what matched.
 */

/** Case, accents and spacing fold away, so a query without diacritics still matches. */
export function normalizeSearchText(value: string): string {
  return value.normalize('NFKD').replace(/\p{M}/gu, '').toLowerCase().replace(/\s+/g, ' ').trim();
}

export interface SearchQuery {
  phrase: string;
  tokens: string[];
}

export function searchQuery(raw: string): SearchQuery {
  const phrase = normalizeSearchText(raw);
  return { phrase, tokens: phrase ? phrase.split(' ') : [] };
}

/**
 * How well `title` and the further `terms` answer the query, or `null` when a
 * word is missing from all of them. The title outranks every other field.
 */
export function rankSearchMatch(query: SearchQuery, title: string, terms: readonly string[] = []): number | null {
  if (query.tokens.length === 0) return 0;
  const normalizedTitle = normalizeSearchText(title);
  const fields = [normalizedTitle, ...terms.map(normalizeSearchText)];
  if (!query.tokens.every((token) => fields.some((field) => field.includes(token)))) return null;
  if (normalizedTitle === query.phrase) return 6;
  if (normalizedTitle.startsWith(query.phrase)) return 5;
  if (normalizedTitle.split(' ').some((word) => word.startsWith(query.phrase))) return 4;
  if (normalizedTitle.includes(query.phrase)) return 3;
  if (query.tokens.every((token) => normalizedTitle.includes(token))) return 2;
  if (fields.some((field) => field.includes(query.phrase))) return 1;
  return 0;
}

interface RankedEntry {
  id: string;
  title: string;
  keywords?: string;
}

/**
 * Entries answering `query`, best first. Among equal matches the one used
 * most recently wins, then the original order.
 */
export function rankByQuery<T extends RankedEntry>(
  entries: readonly T[],
  query: SearchQuery,
  recentIds: readonly string[] = [],
): T[] {
  const recency = (id: string) => {
    const index = recentIds.indexOf(id);
    return index < 0 ? Number.POSITIVE_INFINITY : index;
  };
  return entries
    .flatMap((entry, order) => {
      const rank = rankSearchMatch(query, entry.title, entry.keywords ? [entry.keywords] : []);
      return rank === null ? [] : [{ entry, rank, order }];
    })
    .sort((left, right) => right.rank - left.rank
      || recency(left.entry.id) - recency(right.entry.id)
      || left.order - right.order)
    .map(({ entry }) => entry);
}
