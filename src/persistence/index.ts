/**
 * Small synchronous persistence boundary for device-local preferences.
 *
 * Storage is resolved lazily so importing a module that owns a persisted value
 * remains safe in the viewer, tray, SSR-like tests, and native startup paths.
 */

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export type StorageProvider = () => StorageLike | null | undefined;

export type PersistenceDecoder<T> = (raw: string) => T | undefined;
export type PersistenceEncoder<T> = (value: T) => string;

export interface PersistedSlot<T> {
  readonly key: string;
  readonly fallback: () => T;
  readonly decode: PersistenceDecoder<T>;
  readonly encode: PersistenceEncoder<T>;
}

export interface PersistedSlotOptions<T> {
  fallback: () => T;
  decode: PersistenceDecoder<T>;
  encode: PersistenceEncoder<T>;
}

export interface Persistence {
  read<T>(slot: PersistedSlot<T>): T;
  write<T>(slot: PersistedSlot<T>, value: T): boolean;
  remove<T>(slot: PersistedSlot<T>): boolean;
}

function browserStorage(): StorageLike | null {
  if (typeof window === 'undefined') return null;

  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

let storageProvider: StorageProvider = browserStorage;

function resolveStorage(): StorageLike | null {
  try {
    return storageProvider() ?? null;
  } catch {
    return null;
  }
}

/**
 * Replaces the storage lookup for tests and platform adapters.
 *
 * The provider itself is retained, rather than a storage object, so the
 * storage getter is not touched until a read, write, or remove is attempted.
 * The returned function restores the previous provider.
 */
export function setPersistenceStorageProvider(provider: StorageProvider): () => void {
  const previous = storageProvider;
  storageProvider = provider;
  return () => {
    storageProvider = previous;
  };
}

export const persistence: Persistence = {
  read<T>(slot: PersistedSlot<T>): T {
    const storage = resolveStorage();
    if (!storage) return slot.fallback();

    let raw: string | null;
    try {
      raw = storage.getItem(slot.key);
    } catch {
      return slot.fallback();
    }

    if (raw === null) return slot.fallback();

    try {
      const decoded = slot.decode(raw);
      return decoded === undefined ? slot.fallback() : decoded;
    } catch {
      return slot.fallback();
    }
  },

  write<T>(slot: PersistedSlot<T>, value: T): boolean {
    const storage = resolveStorage();
    if (!storage) return false;

    let encoded: string;
    try {
      encoded = slot.encode(value);
    } catch {
      return false;
    }

    try {
      storage.setItem(slot.key, encoded);
      return true;
    } catch {
      return false;
    }
  },

  remove<T>(slot: PersistedSlot<T>): boolean {
    const storage = resolveStorage();
    if (!storage) return false;

    try {
      storage.removeItem(slot.key);
      return true;
    } catch {
      return false;
    }
  },
};

export function createPersistedSlot<T>(
  key: string,
  options: PersistedSlotOptions<T>,
): PersistedSlot<T> {
  return {
    key,
    ...options,
  };
}
