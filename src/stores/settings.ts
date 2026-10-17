import { ref, watch } from 'vue';
import { defineStore } from 'pinia';
import { normalizeDomain } from '@/utils/senderDomains';
import { linkRegistrableDomain } from '@/utils/externalLinks';
import {
  DEFAULT_SWIPE_PREFERENCES,
  type SwipeActionId,
  type SwipePreferences,
  type SwipeSlot,
} from '@/utils/swipeActions';
import { DEFAULT_KEYBINDINGS } from '@/config/keybindings';
import {
  isLocalePreference,
  readStoredLocalePreference,
  resolveLocale,
  type AppLocale,
  type LocalePreference,
} from '@/i18n/locale';
import { applyLocalePreference } from '@/i18n/runtime';
import { hasStoredTheme, readTheme, writeTheme } from '@/persistence/theme';
import { settingsPersistence } from '@/persistence/settings';
import type {
  AttachmentOpenBehavior,
  InboxGroupingMode,
  InboxSenderBucket,
  Theme,
} from '@/types/settings';

export type { AttachmentOpenBehavior, InboxGroupingMode, InboxSenderBucket } from '@/types/settings';

function applyTheme(theme: Theme) {
  if (typeof document === 'undefined') return;
  document.documentElement.dataset.theme = theme;
  writeTheme(theme);
}

export const useSettingsStore = defineStore('settings', () => {
  const theme = ref<Theme>(readTheme());
  applyTheme(theme.value);
  watch(theme, (next) => applyTheme(next));

  // React to OS theme changes when user hasn't explicitly chosen.
  if (typeof window !== 'undefined' && window.matchMedia) {
    const mql = window.matchMedia('(prefers-color-scheme: light)');
    mql.addEventListener?.('change', (event) => {
      if (!hasStoredTheme()) {
        theme.value = event.matches ? 'light' : 'dark';
      }
    });
  }

  /*
   * The i18n instance already read this preference at module load — the store
   * only owns changing it. `system` re-resolves against the OS languages, so a
   * user who never chose stays on whatever the machine is set to.
   */
  const localePreference = ref<LocalePreference>(readStoredLocalePreference());
  const locale = ref<AppLocale>(resolveLocale(localePreference.value));

  function setLocalePreference(preference: LocalePreference) {
    if (!isLocalePreference(preference)) return;
    localePreference.value = preference;
    locale.value = resolveLocale(preference);
    applyLocalePreference(preference);
  }

  /**
   * The master switch for the bare-key shortcuts (j/k, e, #, g …). The command
   * palette and the shortcut help sit outside it on purpose — they are how a
   * user who turned this off still finds their way around.
   */
  const singleKeyShortcuts = ref(settingsPersistence.singleKeyShortcuts.read());
  watch(singleKeyShortcuts, (next) => settingsPersistence.singleKeyShortcuts.write(next));
  const thirdPartyContactAvatars = ref(settingsPersistence.thirdPartyContactAvatars.read());
  watch(thirdPartyContactAvatars, (next) => settingsPersistence.thirdPartyContactAvatars.write(next));
  const onboardingComplete = ref(settingsPersistence.onboardingComplete.read());
  const autoDarkHtmlMails = ref(settingsPersistence.autoDarkHtmlMails.read());
  watch(autoDarkHtmlMails, (next) => settingsPersistence.autoDarkHtmlMails.write(next));

  /** Render every message as plain text; HTML stays one click away per mail. */
  const preferPlainTextReading = ref(settingsPersistence.preferPlainTextReading.read());
  watch(preferPlainTextReading, (next) =>
    settingsPersistence.preferPlainTextReading.write(next),
  );

  /**
   * Reader text scale in percent. Senders routinely ship 10px body text;
   * without this the only remedy is the OS-wide zoom.
   */
  const readerZoom = ref(settingsPersistence.readerZoom.read());
  watch(readerZoom, (next) => settingsPersistence.readerZoom.write(next));

  function setReaderZoom(percent: number) {
    // Below ~70% lines collapse; above 200% tables break out of the pane.
    readerZoom.value = Math.min(200, Math.max(70, Math.round(percent)));
  }

  /** Compose without an HTML body (text/plain only). */
  const composePlainText = ref(settingsPersistence.composePlainText.read());
  watch(composePlainText, (next) => settingsPersistence.composePlainText.write(next));
  const collapseQuotedHistory = ref(settingsPersistence.collapseQuotedHistory.read());
  watch(collapseQuotedHistory, (next) => settingsPersistence.collapseQuotedHistory.write(next));
  const attachmentOpenBehavior = ref<AttachmentOpenBehavior>(
    settingsPersistence.attachmentOpenBehavior.read(),
  );
  watch(attachmentOpenBehavior, (next) => settingsPersistence.attachmentOpenBehavior.write(next));
  const inboxGroupingMode = ref<InboxGroupingMode>(settingsPersistence.inboxGroupingMode.read());
  watch(inboxGroupingMode, (next) => settingsPersistence.inboxGroupingMode.write(next));
  const swipeActions = ref<SwipePreferences>(settingsPersistence.swipeActions.read());
  watch(swipeActions, (next) => settingsPersistence.swipeActions.write(next), { deep: true });
  /** Contacts pinned to the top of the sidebar's Häufig list (lowercased emails). */
  const pinnedContacts = ref<string[]>(settingsPersistence.pinnedContacts.read());
  watch(pinnedContacts, (next) => settingsPersistence.pinnedContacts.write(next), { deep: true });

  function togglePinnedContact(email: string) {
    const normalized = email.trim().toLowerCase();
    if (!normalized || !normalized.includes('@')) return;
    pinnedContacts.value = pinnedContacts.value.includes(normalized)
      ? pinnedContacts.value.filter((entry) => entry !== normalized)
      : [...pinnedContacts.value, normalized];
  }

  const inboxSenderBuckets = ref<Record<string, InboxSenderBucket>>(
    settingsPersistence.inboxSenderBuckets.read(),
  );
  watch(
    inboxSenderBuckets,
    (next) => settingsPersistence.inboxSenderBuckets.write(next),
    { deep: true },
  );
  const acceptedSenders = ref<Record<string, true>>(settingsPersistence.acceptedSenders.read());
  watch(acceptedSenders, (next) => settingsPersistence.acceptedSenders.write(next), { deep: true });
  const acceptedDomains = ref<Record<string, true>>(settingsPersistence.acceptedDomains.read());
  watch(acceptedDomains, (next) => settingsPersistence.acceptedDomains.write(next), { deep: true });
  /** Link hostnames the user chose to open without the confirm prompt. */
  const trustedLinkHosts = ref<Record<string, true>>(settingsPersistence.trustedLinkHosts.read());
  watch(trustedLinkHosts, (next) => settingsPersistence.trustedLinkHosts.write(next), { deep: true });
  const senderReviewStartedAt = ref<string | null>(settingsPersistence.senderReviewStartedAt.read());
  const dismissedNewSenders = ref<Record<string, string>>(
    settingsPersistence.dismissedNewSenders.read(),
  );

  const keybindings = ref<Record<string, string>>(settingsPersistence.keybindings.read());
  watch(keybindings, (next) => settingsPersistence.keybindings.write(next), { deep: true });

  function setKeybinding(action: string, binding: string) {
    if (!(action in DEFAULT_KEYBINDINGS) || !binding.trim()) return;
    keybindings.value = { ...keybindings.value, [action]: binding.trim() };
  }

  function resetKeybindings() {
    keybindings.value = { ...DEFAULT_KEYBINDINGS };
  }

  function toggleTheme() {
    theme.value = theme.value === 'dark' ? 'light' : 'dark';
  }

  function completeOnboarding() {
    onboardingComplete.value = true;
    settingsPersistence.onboardingComplete.write(true);
  }

  function resetOnboarding() {
    onboardingComplete.value = false;
    settingsPersistence.onboardingComplete.write(false);
  }

  function setInboxGroupingMode(nextMode: InboxGroupingMode) {
    inboxGroupingMode.value = nextMode;
  }

  function setSwipeAction(slot: SwipeSlot, action: SwipeActionId) {
    swipeActions.value = { ...swipeActions.value, [slot]: action };
  }

  function resetSwipeActions() {
    swipeActions.value = { ...DEFAULT_SWIPE_PREFERENCES };
  }

  function setSenderInboxBucket(email: string, bucket: InboxSenderBucket | null) {
    const normalized = email.trim().toLowerCase();
    if (!normalized) {
      return;
    }

    if (bucket === null) {
      const next = { ...inboxSenderBuckets.value };
      delete next[normalized];
      inboxSenderBuckets.value = next;
      return;
    }

    inboxSenderBuckets.value = {
      ...inboxSenderBuckets.value,
      [normalized]: bucket,
    };
  }

  function acceptSender(email: string) {
    const normalized = email.trim().toLowerCase();
    if (!normalized) {
      return;
    }

    const next: Record<string, true> = {
      ...acceptedSenders.value,
      [normalized]: true,
    };
    settingsPersistence.acceptedSenders.write(next);
    acceptedSenders.value = next;
  }

  function acceptSenders(emails: string[]) {
    const next = { ...acceptedSenders.value };
    for (const email of emails) {
      const normalized = email.trim().toLowerCase();
      if (normalized) {
        next[normalized] = true;
      }
    }
    settingsPersistence.acceptedSenders.write(next);
    acceptedSenders.value = next;
  }

  function acceptDomain(domain: string) {
    const normalized = normalizeDomain(domain);
    if (!normalized) {
      return;
    }

    const next: Record<string, true> = {
      ...acceptedDomains.value,
      [normalized]: true,
    };
    settingsPersistence.acceptedDomains.write(next);
    acceptedDomains.value = next;
  }

  function forgetAcceptedDomain(domain: string) {
    const normalized = normalizeDomain(domain);
    if (!normalized) {
      return;
    }

    const next = { ...acceptedDomains.value };
    delete next[normalized];
    settingsPersistence.acceptedDomains.write(next);
    acceptedDomains.value = next;
  }

  /**
   * Trust covers the registrable domain, not the exact host: senders rotate
   * subdomains freely (`click2.mailings.…`), and a prompt for each would make
   * "künftig ohne Nachfrage" worthless. IPs and localhost stay exact.
   */
  function isTrustedLinkHost(hostname: string): boolean {
    const domain = linkRegistrableDomain(hostname);
    return Boolean(domain && trustedLinkHosts.value[domain]);
  }

  function trustLinkHost(hostname: string) {
    const domain = linkRegistrableDomain(hostname);
    if (!domain) return;
    const next: Record<string, true> = { ...trustedLinkHosts.value, [domain]: true };
    settingsPersistence.trustedLinkHosts.write(next);
    trustedLinkHosts.value = next;
  }

  function forgetTrustedLinkHost(hostname: string) {
    const domain = linkRegistrableDomain(hostname);
    if (!domain) return;
    const next = { ...trustedLinkHosts.value };
    delete next[domain];
    settingsPersistence.trustedLinkHosts.write(next);
    trustedLinkHosts.value = next;
  }

  function completeSenderReviewSetup(existingEmails: string[]) {
    acceptSenders(existingEmails);
    const startedAt = new Date().toISOString();
    settingsPersistence.senderReviewStartedAt.write(startedAt);
    senderReviewStartedAt.value = startedAt;
  }

  function setDismissedNewSender(email: string, throughDate: string | null) {
    const normalized = email.trim().toLowerCase();
    if (!normalized) return;
    const next = { ...dismissedNewSenders.value };
    if (throughDate && Number.isFinite(Date.parse(throughDate))) {
      next[normalized] = throughDate;
    } else {
      delete next[normalized];
    }
    settingsPersistence.dismissedNewSenders.write(next);
    dismissedNewSenders.value = next;
  }

  function forgetAcceptedSender(email: string) {
    const normalized = email.trim().toLowerCase();
    if (!normalized) {
      return;
    }

    const next = { ...acceptedSenders.value };
    delete next[normalized];
    settingsPersistence.acceptedSenders.write(next);
    acceptedSenders.value = next;
  }

  return {
    theme,
    locale,
    localePreference,
    setLocalePreference,
    singleKeyShortcuts,
    onboardingComplete,
    autoDarkHtmlMails,
    preferPlainTextReading,
    composePlainText,
    readerZoom,
    setReaderZoom,
    collapseQuotedHistory,
    thirdPartyContactAvatars,
    attachmentOpenBehavior,
    inboxGroupingMode,
    swipeActions,
    inboxSenderBuckets,
    acceptedSenders,
    acceptedDomains,
    trustedLinkHosts,
    isTrustedLinkHost,
    trustLinkHost,
    forgetTrustedLinkHost,
    senderReviewStartedAt,
    dismissedNewSenders,
    keybindings,
    setKeybinding,
    resetKeybindings,
    pinnedContacts,
    togglePinnedContact,
    toggleTheme,
    completeOnboarding,
    resetOnboarding,
    setInboxGroupingMode,
    setSwipeAction,
    resetSwipeActions,
    setSenderInboxBucket,
    acceptSender,
    acceptSenders,
    acceptDomain,
    forgetAcceptedDomain,
    completeSenderReviewSetup,
    setDismissedNewSender,
    forgetAcceptedSender,
  };
});
