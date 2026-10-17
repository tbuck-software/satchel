import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useRecentSearches } from './useRecentSearches';

const STORAGE_KEY = 'satchel.search.recent';

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
    clear: () => values.clear(),
  };
}

describe('useRecentSearches', () => {
  beforeEach(() => {
    vi.stubGlobal('window', { localStorage: memoryStorage() });
    useRecentSearches().clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('stores queries most-recent-first under the satchel key', () => {
    const { recent, push } = useRecentSearches();
    push('rechnung');
    push('from:anna@example.com');

    expect(recent.value).toEqual(['from:anna@example.com', 'rechnung']);
    expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? '[]')).toEqual([
      'from:anna@example.com',
      'rechnung',
    ]);
  });

  it('dedupes case-insensitively and moves the entry to the front', () => {
    const { recent, push } = useRecentSearches();
    push('Rechnung');
    push('anna');
    push('rechnung');

    expect(recent.value).toEqual(['rechnung', 'anna']);
  });

  it('caps the list at eight entries', () => {
    const { recent, push } = useRecentSearches();
    for (let i = 0; i < 12; i += 1) {
      push(`suche-${i}`);
    }

    expect(recent.value).toHaveLength(8);
    expect(recent.value[0]).toBe('suche-11');
  });

  it('removes a single entry case-insensitively', () => {
    const { recent, push, remove } = useRecentSearches();
    push('rechnung');
    push('anna');
    remove('Rechnung');

    expect(recent.value).toEqual(['anna']);
    expect(JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? '[]')).toEqual(['anna']);
  });

  it('ignores blank queries', () => {
    const { recent, push } = useRecentSearches();
    push('   ');

    expect(recent.value).toEqual([]);
  });

  it('keeps updates safe when storage rejects writes', () => {
    const setItem = vi.fn(() => {
      throw new Error('set denied');
    });
    vi.stubGlobal('window', {
      localStorage: {
        getItem: () => null,
        setItem,
        removeItem: () => undefined,
      },
    });
    const { recent, push } = useRecentSearches();

    expect(() => push('rechnung')).not.toThrow();
    expect(recent.value).toEqual(['rechnung']);
    expect(setItem).toHaveBeenCalledOnce();
  });
});
