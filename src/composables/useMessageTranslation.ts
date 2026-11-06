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
  'backend.errors.translation.gmailNeedsPro',
]);

/**
 * The messages on screen that can be translated, by id, with the toggle each
 * one runs. The palette and the `L` key reach a message through this, the
 * way the reader's other commands reach the mounted thread.
 */
const togglers = shallowReactive(new Map<string, () => void>());

/** Runs Translate / Show original for a mounted message; false if it can do neither. */
export function toggleMessageTranslation(messageId: string): boolean {
  const toggle = togglers.get(messageId);
  if (!toggle) return false;
  toggle();
  return true;
}

/** Whether a mounted message can be translated (or is showing a translation). */
export function messageCanTranslate(messageId: string): boolean {
  return togglers.has(messageId);
}

export interface MessageTranslationSource {
  id: string;
  accountId: string;
  bodyLoaded: boolean;
  /** PGP or S/MIME mail: never sent anywhere. */
  encrypted: boolean;
  /** A folded message shows no body; it registers nothing and asks nothing. */
  collapsed: boolean;
}

/**
 * One message's translation: whether it can be translated, whether the
 * header should offer it, the translated body while it is shown, and the way
 * back to the original.
 *
 * Two levels. `available` is everything that has to hold for Translate to
 * work at all: an engine with a key, a loaded body, not encrypted, not
 * folded. That is when `L`, the palette and the phone sheet offer it. The
 * local language guess only decides `offered`, the header button: mail that
 * already looks like the target language gets no button, but can still be
 * translated on request, because the guess can be wrong.
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

  const available = computed(() => {
    const current = source();
    return store.ready && current.bodyLoaded && !current.encrypted && !current.collapsed;
  });

  watch(
    () => [source().id, store.targetLanguage, available.value] as const,
    async ([id, target, isAvailable], previous) => {
      if (previous && previous[0] !== id) {
        translation.value = null;
        showing.value = false;
      }
      const request = ++offerRequest;
      if (!isAvailable) {
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

  /** The header button: available, and the guess says it is worth it. */
  const offered = computed(() => shown.value !== null || (available.value && Boolean(offer.value?.offered)));

  function report(key: string | null, message: string) {
    toast.error(message, key && SETTINGS_ERRORS.has(key)
      ? { action: { label: t('translation.reader.openSettings'), handler: () => ui.openSettings('translation') } }
      : {});
  }

  async function translate() {
    const current = source();
    if (busy.value || !available.value) return;
    if (translation.value?.target_language === store.targetLanguage) {
      showing.value = true;
      return;
    }
    // Refused whatever the answer to a consent question would be; say why
    // instead of asking.
    const refusal = offer.value?.refusal ?? null;
    if (refusal && !offer.value?.cached) {
      report(refusal, t(refusal));
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
      report(backendErrorKey(error), error instanceof Error ? error.message : String(error));
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

  let registeredId: string | null = null;
  function unregister() {
    if (registeredId && togglers.get(registeredId) === toggle) togglers.delete(registeredId);
    registeredId = null;
  }
  watch(
    () => [source().id, available.value] as const,
    ([id, isRegistered]) => {
      if (registeredId && (registeredId !== id || !isRegistered)) unregister();
      if (isRegistered && registeredId !== id) {
        togglers.set(id, toggle);
        registeredId = id;
      }
    },
    { immediate: true },
  );
  onBeforeUnmount(unregister);

  return { available, offered, shown, busy, translate, showOriginal, toggle };
}
