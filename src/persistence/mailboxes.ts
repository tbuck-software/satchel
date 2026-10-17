import { createPersistedSlot, persistence } from './index';

const FAVORITES_STORAGE_KEY = 'satchel.mailboxes.favoriteFolders';

const favoriteFolderKeysSlot = createPersistedSlot<string[]>(FAVORITES_STORAGE_KEY, {
  fallback: () => [],
  decode: (raw) => {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return undefined;
    }

    return parsed
      .map((value) => (typeof value === 'string' ? value.trim() : ''))
      .filter((value) => Boolean(value) && value.includes('::'));
  },
  encode: (value) => {
    const normalized = Array.from(
      new Set(value.filter((entry) => entry.includes('::')).filter(Boolean)),
    ).sort((left, right) =>
      left.localeCompare(right, undefined, { sensitivity: 'base' }),
    );
    return JSON.stringify(normalized);
  },
});

export const mailboxesPersistence = {
  favoriteFolderKeys: {
    read: () => persistence.read(favoriteFolderKeysSlot),
    write: (value: string[]) => {
      persistence.write(favoriteFolderKeysSlot, value);
    },
  },
};
