import { createPersistedSlot, persistence } from './index';

const PERF_ENABLED_STORAGE_KEY = 'satchel:perf';

const enabledSlot = createPersistedSlot<boolean>(PERF_ENABLED_STORAGE_KEY, {
  fallback: () => false,
  decode: (raw) => raw === '1',
  encode: (value) => (value ? '1' : '0'),
});

export const perfPersistence = {
  enabled: {
    read: () => persistence.read(enabledSlot),
  },
};
