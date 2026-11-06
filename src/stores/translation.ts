import { defineStore } from 'pinia';
import { computed, shallowRef } from 'vue';
import { backendCommandsAvailable } from '@/services/backend/core';
import {
  backendGetTranslationSettings,
  backendSetTranslationConsent,
  type BackendTranslationEngineId,
  type BackendTranslationSettings,
} from '@/services/backend/translation';
import { currentLocale } from '@/i18n/runtime';
import type { AppLocale } from '@/i18n/locale';

/**
 * The language mail is translated into while the setting follows the UI.
 * DeepL wants a variant for English; American is its most common one.
 */
const UI_TARGET: Readonly<Record<AppLocale, string>> = { en: 'EN-US', de: 'DE' };

export function uiTargetLanguage(locale: AppLocale): string {
  return UI_TARGET[locale] ?? 'EN-US';
}

/**
 * Translation as this device has it set up: which engine, into which
 * language, and which accounts' mail may go there. Loaded with the first
 * reader that asks, not at startup; the settings page writes back what the
 * backend returns after each change.
 */
export const useTranslationStore = defineStore('translation', () => {
  const settings = shallowRef<BackendTranslationSettings | null>(null);
  let loading: Promise<void> | null = null;

  async function reload() {
    if (!backendCommandsAvailable()) return;
    try {
      settings.value = await backendGetTranslationSettings();
    } catch {
      // Without settings the reader simply offers no Translate.
    }
  }

  function ensureLoaded(): Promise<void> {
    loading ??= reload();
    return loading;
  }

  function apply(next: BackendTranslationSettings) {
    settings.value = next;
  }

  const engine = computed(() => (settings.value?.engine ?? null) as BackendTranslationEngineId | null);

  /** An engine is switched on and has a key: Translate can work. */
  const ready = computed(() => {
    const id = engine.value;
    return id !== null && Boolean(settings.value?.engines.find((entry) => entry.id === id)?.has_key);
  });

  /** The stored key's plan (DeepL `free` / `pro`), or null without one. */
  const plan = computed(() => {
    const id = engine.value;
    return settings.value?.engines.find((entry) => entry.id === id)?.plan ?? null;
  });

  const targetLanguage = computed(() => settings.value?.target_language ?? uiTargetLanguage(currentLocale()));

  function consented(accountId: string): boolean {
    const id = engine.value;
    return id !== null && Boolean(settings.value?.consents.some(
      (consent) => consent.engine === id && consent.account_id === accountId,
    ));
  }

  async function setConsent(accountId: string, granted: boolean) {
    const id = engine.value;
    if (!id) return;
    settings.value = await backendSetTranslationConsent(id, accountId, granted);
  }

  return { settings, engine, ready, plan, targetLanguage, ensureLoaded, reload, apply, consented, setConsent };
});
