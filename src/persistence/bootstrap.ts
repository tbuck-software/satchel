import { createPersistedSlot, persistence } from './index';

const LAST_SIGNATURE_DISCOVERY_AT_STORAGE_KEY = 'satchel.signatures.lastDiscoveryAt';

const lastSignatureDiscoveryAtSlot = createPersistedSlot<number>(
  LAST_SIGNATURE_DISCOVERY_AT_STORAGE_KEY,
  {
    fallback: () => 0,
    decode: (raw) => Number(raw),
    encode: (value) => String(value),
  },
);

export const bootstrapPersistence = {
  lastSignatureDiscoveryAt: {
    read: () => persistence.read(lastSignatureDiscoveryAtSlot),
    write: (value: number) => {
      persistence.write(lastSignatureDiscoveryAtSlot, value);
    },
  },
};
