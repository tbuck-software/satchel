import { MAX_RECENT_COMMANDS, MAX_RECENT_SEARCHES } from '@/config/historyLimits';
import { createPersistedSlot, persistence } from './index';

const RECENT_SEARCHES_STORAGE_KEY = 'satchel.search.recent';
const RECENT_COMMANDS_STORAGE_KEY = 'satchel.commands.recent';

function stringList(limit: number) {
  return (raw: string) => {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((entry): entry is string => typeof entry === 'string').slice(0, limit)
      : undefined;
  };
}

const recentQueriesSlot = createPersistedSlot<string[]>(RECENT_SEARCHES_STORAGE_KEY, {
  fallback: () => [],
  decode: stringList(MAX_RECENT_SEARCHES),
  encode: (value) => JSON.stringify(value),
});

const recentCommandsSlot = createPersistedSlot<string[]>(RECENT_COMMANDS_STORAGE_KEY, {
  fallback: () => [],
  decode: stringList(MAX_RECENT_COMMANDS),
  encode: (value) => JSON.stringify(value),
});

export const searchPersistence = {
  recentCommands: {
    read: () => persistence.read(recentCommandsSlot),
    write: (value: string[]) => {
      persistence.write(recentCommandsSlot, value);
    },
  },
  recentQueries: {
    read: () => persistence.read(recentQueriesSlot),
    write: (value: string[]) => {
      persistence.write(recentQueriesSlot, value);
    },
  },
};
