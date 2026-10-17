import type { ImagePolicyMode } from '@/types/settings';
import { createPersistedSlot, persistence } from './index';

const ALLOWED_SENDERS_STORAGE_KEY = 'satchel:images-allowed-senders';
const ALLOWED_DOMAINS_STORAGE_KEY = 'satchel:images-allowed-domains';
const MODE_STORAGE_KEY = 'satchel:images-policy-mode';
const LEGACY_ALLOW_ALL_STORAGE_KEY = 'satchel:images-allow-all';

function stringSetSlot(key: string) {
  return createPersistedSlot<Set<string>>(key, {
    fallback: () => new Set<string>(),
    decode: (raw) => {
      const parsed: unknown = JSON.parse(raw);
      if (!Array.isArray(parsed)) return undefined;
      return new Set(parsed.map((value) => {
        if (typeof value !== 'string') throw new TypeError('Expected a string');
        return value.toLowerCase();
      }));
    },
    encode: (value) => JSON.stringify(Array.from(value)),
  });
}

const allowedSendersSlot = stringSetSlot(ALLOWED_SENDERS_STORAGE_KEY);
const allowedDomainsSlot = stringSetSlot(ALLOWED_DOMAINS_STORAGE_KEY);

const modeSlot = createPersistedSlot<ImagePolicyMode | null>(MODE_STORAGE_KEY, {
  fallback: () => null,
  decode: (raw) => (
    raw === 'allowlist' || raw === 'accepted' || raw === 'all' ? raw : null
  ),
  encode: (value) => value ?? '',
});

const legacyAllowAllSlot = createPersistedSlot<boolean>(LEGACY_ALLOW_ALL_STORAGE_KEY, {
  fallback: () => false,
  decode: (raw) => raw === 'true',
  encode: (value) => String(value),
});

function readMode(): ImagePolicyMode {
  const mode = persistence.read(modeSlot);
  if (mode) return mode;
  return persistence.read(legacyAllowAllSlot) ? 'all' : 'allowlist';
}

function writeMode(mode: ImagePolicyMode) {
  if (persistence.write(modeSlot, mode)) {
    persistence.remove(legacyAllowAllSlot);
  }
}

export const imagePolicyPersistence = {
  allowedSenders: {
    read: () => persistence.read(allowedSendersSlot),
    write: (value: Set<string>) => {
      persistence.write(allowedSendersSlot, value);
    },
  },
  allowedDomains: {
    read: () => persistence.read(allowedDomainsSlot),
    write: (value: Set<string>) => {
      persistence.write(allowedDomainsSlot, value);
    },
  },
  mode: {
    read: readMode,
    write: writeMode,
  },
};
