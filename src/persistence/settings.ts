import { DEFAULT_KEYBINDINGS } from '@/config/keybindingDefaults';
import type { AttachmentOpenBehavior, InboxGroupingMode, InboxSenderBucket } from '@/types/settings';
import { linkRegistrableDomain } from '@/utils/externalLinks';
import { normalizeDomain } from '@/utils/senderDomains';
import {
  DEFAULT_SWIPE_PREFERENCES,
  isSwipeActionId,
  type SwipePreferences,
} from '@/utils/swipeActions';
import {
  createPersistedSlot,
  persistence,
  type PersistedSlot,
} from './index';

const SETTINGS_STORAGE_KEYS = {
  thirdPartyContactAvatars: 'satchel.thirdPartyContactAvatars',
  onboardingComplete: 'satchel.onboardingComplete',
  singleKeyShortcuts: 'satchel.singleKeyShortcuts',
  autoDarkHtmlMails: 'satchel.autoDarkHtmlMails',
  preferPlainTextReading: 'satchel.preferPlainTextReading',
  readerZoom: 'satchel.readerZoom',
  composePlainText: 'satchel.composePlainText',
  collapseQuotedHistory: 'satchel.collapseQuotedHistory',
  attachmentOpenBehavior: 'satchel.attachmentOpenBehavior',
  inboxGroupingMode: 'satchel.inboxGroupingMode',
  swipeActions: 'satchel.swipeActions',
  pinnedContacts: 'satchel.pinnedContacts',
  inboxSenderBuckets: 'satchel.inboxSenderBuckets',
  acceptedSenders: 'satchel.acceptedSenders',
  acceptedDomains: 'satchel.acceptedDomains',
  trustedLinkHosts: 'satchel.trustedLinkHosts',
  senderReviewStartedAt: 'satchel.senderReviewStartedAt',
  dismissedNewSenders: 'satchel.dismissedNewSenders',
  keybindings: 'satchel.keybindings',
} as const;

export interface PersistedSetting<T> {
  read(): T;
  write(value: T): void;
}

function setting<T>(slot: PersistedSlot<T>): PersistedSetting<T> {
  return {
    read: () => persistence.read(slot),
    write: (value) => {
      persistence.write(slot, value);
    },
  };
}

function parseJson(raw: string): unknown {
  return JSON.parse(raw);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

const booleanSlot = (key: string, fallback: boolean) => createPersistedSlot(key, {
  fallback: () => fallback,
  // Keep the legacy behavior: every present value other than "true" means false.
  decode: (raw) => raw === 'true',
  encode: (value: boolean) => String(value),
});

const onboardingCompleteSlot = booleanSlot(SETTINGS_STORAGE_KEYS.onboardingComplete, false);
const singleKeyShortcutsSlot = booleanSlot(SETTINGS_STORAGE_KEYS.singleKeyShortcuts, true);
const autoDarkHtmlMailsSlot = booleanSlot(SETTINGS_STORAGE_KEYS.autoDarkHtmlMails, true);
const preferPlainTextReadingSlot = booleanSlot(SETTINGS_STORAGE_KEYS.preferPlainTextReading, false);
const composePlainTextSlot = booleanSlot(SETTINGS_STORAGE_KEYS.composePlainText, false);
const collapseQuotedHistorySlot = booleanSlot(SETTINGS_STORAGE_KEYS.collapseQuotedHistory, true);

const readerZoomSlot = createPersistedSlot<number>(SETTINGS_STORAGE_KEYS.readerZoom, {
  fallback: () => 100,
  decode: (raw) => {
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : undefined;
  },
  encode: (value) => String(value),
});

const attachmentOpenBehaviorSlot = createPersistedSlot<AttachmentOpenBehavior>(
  SETTINGS_STORAGE_KEYS.attachmentOpenBehavior,
  {
    fallback: () => 'preview',
    decode: (raw) => raw === 'system' ? 'system' : 'preview',
    encode: (value) => value,
  },
);

const inboxGroupingModeSlot = createPersistedSlot<InboxGroupingMode>(
  SETTINGS_STORAGE_KEYS.inboxGroupingMode,
  {
    fallback: () => 'smart',
    decode: (raw) => raw === 'classic' || raw === 'smart' ? raw : 'smart',
    encode: (value) => value,
  },
);

const keybindingsSlot = createPersistedSlot<Record<string, string>>(
  SETTINGS_STORAGE_KEYS.keybindings,
  {
    fallback: () => ({ ...DEFAULT_KEYBINDINGS }),
    decode: (raw) => {
      const parsed = parseJson(raw);
      if (!isRecord(parsed)) return undefined;

      const defaults = { ...DEFAULT_KEYBINDINGS };
      for (const action of Object.keys(defaults)) {
        const binding = parsed[action];
        if (typeof binding === 'string' && binding.trim()) {
          defaults[action] = binding.trim();
        }
      }
      return defaults;
    },
    encode: (value) => JSON.stringify(value),
  },
);

const swipeActionsSlot = createPersistedSlot<SwipePreferences>(SETTINGS_STORAGE_KEYS.swipeActions, {
  fallback: () => ({ ...DEFAULT_SWIPE_PREFERENCES }),
  decode: (raw) => {
    const parsed = parseJson(raw);
    if (!isRecord(parsed)) return undefined;
    return {
      rightShort: isSwipeActionId(parsed.rightShort)
        ? parsed.rightShort
        : DEFAULT_SWIPE_PREFERENCES.rightShort,
      rightLong: isSwipeActionId(parsed.rightLong)
        ? parsed.rightLong
        : DEFAULT_SWIPE_PREFERENCES.rightLong,
      leftShort: isSwipeActionId(parsed.leftShort)
        ? parsed.leftShort
        : DEFAULT_SWIPE_PREFERENCES.leftShort,
      leftLong: isSwipeActionId(parsed.leftLong)
        ? parsed.leftLong
        : DEFAULT_SWIPE_PREFERENCES.leftLong,
    };
  },
  encode: (value) => JSON.stringify(value),
});

const pinnedContactsSlot = createPersistedSlot<string[]>(SETTINGS_STORAGE_KEYS.pinnedContacts, {
  fallback: () => [],
  decode: (raw) => {
    const parsed = parseJson(raw);
    if (!Array.isArray(parsed)) return undefined;
    return parsed
      .filter((entry): entry is string => typeof entry === 'string')
      .map((entry) => entry.trim().toLowerCase())
      .filter((entry) => entry.includes('@'));
  },
  encode: (value) => JSON.stringify(value),
});

const inboxSenderBucketsSlot = createPersistedSlot<Record<string, InboxSenderBucket>>(
  SETTINGS_STORAGE_KEYS.inboxSenderBuckets,
  {
    fallback: () => ({}),
    decode: (raw) => {
      const parsed = parseJson(raw);
      if (!isRecord(parsed)) return undefined;

      const allowed = new Set<InboxSenderBucket>([
        'personal',
        'notifications',
        'newsletter',
        'social',
      ]);
      const result: Record<string, InboxSenderBucket> = {};
      for (const [key, value] of Object.entries(parsed)) {
        const normalized = key.trim().toLowerCase();
        if (normalized && allowed.has(value as InboxSenderBucket)) {
          result[normalized] = value as InboxSenderBucket;
        }
      }
      return result;
    },
    encode: (value) => JSON.stringify(value),
  },
);

function decodeAcceptedValues(raw: string): string[] | undefined {
  const parsed = parseJson(raw);
  if (Array.isArray(parsed)) return parsed.filter((value): value is string => typeof value === 'string');
  return Object.keys((parsed as Record<string, unknown>) || {});
}

const acceptedSendersSlot = createPersistedSlot<Record<string, true>>(
  SETTINGS_STORAGE_KEYS.acceptedSenders,
  {
    fallback: () => ({}),
    decode: (raw) => {
      const values = decodeAcceptedValues(raw);
      if (!values) return undefined;
      const result: Record<string, true> = {};
      values.forEach((value) => {
        const normalized = value.trim().toLowerCase();
        if (normalized) result[normalized] = true;
      });
      return result;
    },
    encode: (value) => JSON.stringify(Object.keys(value)),
  },
);

const trustedLinkHostsSlot = createPersistedSlot<Record<string, true>>(
  SETTINGS_STORAGE_KEYS.trustedLinkHosts,
  {
    fallback: () => ({}),
    decode: (raw) => {
      const values = decodeAcceptedValues(raw);
      if (!values) return undefined;
      const result: Record<string, true> = {};
      values.forEach((value) => {
        const normalized = linkRegistrableDomain(value);
        if (normalized) result[normalized] = true;
      });
      return result;
    },
    encode: (value) => JSON.stringify(Object.keys(value)),
  },
);

const acceptedDomainsSlot = createPersistedSlot<Record<string, true>>(
  SETTINGS_STORAGE_KEYS.acceptedDomains,
  {
    fallback: () => ({}),
    decode: (raw) => {
      const values = decodeAcceptedValues(raw);
      if (!values) return undefined;
      const result: Record<string, true> = {};
      values.forEach((value) => {
        const normalized = normalizeDomain(value);
        if (normalized) result[normalized] = true;
      });
      return result;
    },
    encode: (value) => JSON.stringify(Object.keys(value)),
  },
);

const senderReviewStartedAtSlot = createPersistedSlot<string | null>(
  SETTINGS_STORAGE_KEYS.senderReviewStartedAt,
  {
    fallback: () => null,
    decode: (raw) => raw && Number.isFinite(Date.parse(raw)) ? raw : null,
    encode: (value) => value ?? '',
  },
);

const dismissedNewSendersSlot = createPersistedSlot<Record<string, string>>(
  SETTINGS_STORAGE_KEYS.dismissedNewSenders,
  {
    fallback: () => ({}),
    decode: (raw) => {
      const parsed = parseJson(raw);
      if (!isRecord(parsed)) return undefined;
      const result: Record<string, string> = {};
      for (const [email, value] of Object.entries(parsed)) {
        const normalized = email.trim().toLowerCase();
        if (normalized && typeof value === 'string' && Number.isFinite(Date.parse(value))) {
          result[normalized] = value;
        }
      }
      return result;
    },
    encode: (value) => JSON.stringify(value),
  },
);

export const settingsPersistence = {
  onboardingComplete: setting(onboardingCompleteSlot),
  singleKeyShortcuts: setting(singleKeyShortcutsSlot),
  autoDarkHtmlMails: setting(autoDarkHtmlMailsSlot),
  preferPlainTextReading: setting(preferPlainTextReadingSlot),
  readerZoom: setting(readerZoomSlot),
  composePlainText: setting(composePlainTextSlot),
  collapseQuotedHistory: setting(collapseQuotedHistorySlot),
  thirdPartyContactAvatars: setting(booleanSlot(SETTINGS_STORAGE_KEYS.thirdPartyContactAvatars, false)),
  attachmentOpenBehavior: setting(attachmentOpenBehaviorSlot),
  inboxGroupingMode: setting(inboxGroupingModeSlot),
  swipeActions: setting(swipeActionsSlot),
  pinnedContacts: setting(pinnedContactsSlot),
  inboxSenderBuckets: setting(inboxSenderBucketsSlot),
  acceptedSenders: setting(acceptedSendersSlot),
  acceptedDomains: setting(acceptedDomainsSlot),
  trustedLinkHosts: setting(trustedLinkHostsSlot),
  senderReviewStartedAt: setting(senderReviewStartedAtSlot),
  dismissedNewSenders: setting(dismissedNewSendersSlot),
  keybindings: setting(keybindingsSlot),
};
