import { computed, onBeforeUnmount, ref, shallowReactive, shallowRef, watch, type Ref } from 'vue';
import {
  backendMessageTranslationOffer,
  backendTranslateMessage,
  type BackendMessageTranslation,
  type BackendMessageTranslationOffer,
} from '@/services/backend/translation';
import { backendErrorKey } from '@/utils/backendErrors';
import { useTranslationStore } from '@/stores/translation';
import { useUiStore } from '@/stores/ui';
import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';

/** What the person can fix in the Translation settings. */
const SETTINGS_ERRORS = new Set([
  'backend.errors.translation.notConfigured',
  'backend.errors.translation.keyMissing',
  'backend.errors.translation.invalidKey',
  'backend.errors.translation.credentialsUnavailable',
  'backend.errors.translation.consentRequired',
]);

/**
 * The messages on screen that offer Translate, by id, with the toggle each
 * one runs. The palette and the `T` key reach a message through this, the
 * way the reader's other commands reach the mounted thread.
 */
const togglers = shallowReactive(new Map<string, () => void>());

/** Runs Translate / Show original for a mounted message; false if it offers neither. */
export function toggleMessageTranslation(messageId: string): boolean {
  const toggle = togglers.get(messageId);
  if (!toggle) return false;
  toggle();
  return true;
}

/** Whether a mounted message offers Translate (or is showing a translation). */
export function messageOffersTranslation(messageId: string): boolean {
  return togglers.has(messageId);
}

export interface MessageTranslationSource {
  id: string;
  accountId: string;
  bodyLoaded: boolean;
  /** PGP or S/MIME mail: never sent anywhere. */
  encrypted: boolean;
}

/**
 * One message's translation: whether to offer it, the translated body while
 * it is shown, and the way back to the original. Asks for consent first when
 * the account has not given it; `requestConsent` shows that question.
 */
export function useMessageTranslation(
  source: () => MessageTranslationSource,
  requestConsent: (accountId: string) => Promise<boolean>,
) {
  const store = useTranslationStore();
  const toast = useToast();
  const ui = useUiStore();
  const offer = shallowRef<BackendMessageTranslationOffer | null>(null);
  const translation = shallowRef<BackendMessageTranslation | null>(null);
  const showing: Ref<boolean> = ref(false);
  const busy = ref(false);
  let offerRequest = 0;

  void store.ensureLoaded();

  const eligible = computed(() => {
    const current = source();
    return store.ready && current.bodyLoaded && !current.encrypted;
  });

  watch(
    () => [source().id, store.targetLanguage, eligible.value] as const,
    async ([id, target, isEligible], previous) => {
      if (previous && previous[0] !== id) {
        translation.value = null;
        showing.value = false;
      }
      const request = ++offerRequest;
      if (!isEligible) {
        offer.value = null;
        return;
      }
      try {
        const next = await backendMessageTranslationOffer(id, target);
        if (request === offerRequest) offer.value = next;
      } catch {
        if (request === offerRequest) offer.value = null;
      }
    },
    { immediate: true },
  );

  /** A translation into a target other than the current one is not shown. */
  const shown = computed(() => (
    showing.value && translation.value !== null && translation.value.target_language === store.targetLanguage
      ? translation.value
      : null
  ));

  const offered = computed(() => shown.value !== null || (eligible.value && Boolean(offer.value?.offered)));

  function report(error: unknown) {
    const key = backendErrorKey(error);
    const message = error instanceof Error ? error.message : String(error);
    toast.error(message, key && SETTINGS_ERRORS.has(key)
      ? { action: { label: t('translation.reader.openSettings'), handler: () => ui.openSettings('translation') } }
      : {});
  }

  async function translate() {
    const current = source();
    if (busy.value || !eligible.value) return;
    if (translation.value?.target_language === store.targetLanguage) {
      showing.value = true;
      return;
    }
    // A stored translation goes without asking: nothing leaves the device.
    if (!offer.value?.cached && !store.consented(current.accountId)) {
      if (!await requestConsent(current.accountId)) return;
    }
    busy.value = true;
    try {
      const result = await backendTranslateMessage(current.id, store.targetLanguage);
      if (source().id !== current.id) return;
      translation.value = result;
      showing.value = true;
    } catch (error) {
      report(error);
    } finally {
      busy.value = false;
    }
  }

  function showOriginal() {
    showing.value = false;
  }

  function toggle() {
    if (shown.value) showOriginal();
    else void translate();
  }

  let registered: string | null = null;
  watch(
    () => [source().id, offered.value] as const,
    ([id, isOffered]) => {
      if (registered && (registered !== id || !isOffered)) {
        if (togglers.get(registered) === toggle) togglers.delete(registered);
        registered = null;
      }
      if (isOffered && registered !== id) {
        togglers.set(id, toggle);
        registered = id;
      }
    },
    { immediate: true },
  );
  onBeforeUnmount(() => {
    if (registered && togglers.get(registered) === toggle) togglers.delete(registered);
  });

  return { offered, shown, busy, translate, showOriginal, toggle };
}
