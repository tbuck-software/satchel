import { createPersistedSlot, persistence } from './index';

/** Enough for every account and a few certificate changes each. */
const MAX_DISMISSED_NOTICES = 32;

/**
 * Certificate notices the person hid, as `<accountId>|<host>|<reason>|<fingerprint>`.
 *
 * Kept across restarts on purpose: the same expired certificate is still the
 * same notice tomorrow, and popping it up again on every launch is the bug
 * this replaces. A different certificate or reason is a new notice.
 */
const dismissedSlot = createPersistedSlot<string[]>('satchel.certificateNotices.dismissed.v1', {
  fallback: () => [],
  decode: (raw) => {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return undefined;
    return parsed
      .filter((value): value is string => typeof value === 'string')
      .slice(-MAX_DISMISSED_NOTICES);
  },
  encode: (value) => JSON.stringify(value.slice(-MAX_DISMISSED_NOTICES)),
});

export const certificateNoticePersistence = {
  dismissed: {
    read: () => persistence.read(dismissedSlot),
    write: (value: string[]) => {
      persistence.write(dismissedSlot, value);
    },
  },
};
