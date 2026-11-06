import type { TranslationSettingsDto as WireTranslationSettings } from './generated/app/commands/translation/TranslationSettingsDto';
import type { TranslationEngineDto as WireTranslationEngine } from './generated/app/commands/translation/TranslationEngineDto';
import type { SaveTranslationSettingsInput } from './generated/app/commands/translation/SaveTranslationSettingsInput';
import type { TranslationUsage as WireTranslationUsage } from './generated/core/translation/TranslationUsage';
import type { TranslationLanguage as WireTranslationLanguage } from './generated/core/translation/TranslationLanguage';
import type { MessageTranslation as WireMessageTranslation } from './generated/core/translation/MessageTranslation';
import type { MessageTranslationOffer as WireMessageTranslationOffer } from './generated/core/translation/MessageTranslationOffer';
import type { RequestLogEntry as WireRequestLogEntry } from './generated/core/db/translation/RequestLogEntry';
import { invoke } from './core';

/** The translation engines Satchel knows. DeepL only, for now. */
export type BackendTranslationEngineId = 'deepl';
/** Engine, target language, consents. The API key itself never comes back. */
export type BackendTranslationSettings = WireTranslationSettings;
export type BackendTranslationEngine = WireTranslationEngine;
export type BackendSaveTranslationSettings = SaveTranslationSettingsInput;
/** What is left of the engine account's allowance this billing period. */
export type BackendTranslationUsage = WireTranslationUsage;
export type BackendTranslationLanguage = WireTranslationLanguage;
/** A message body in another language; `cached` when nothing was sent. */
export type BackendMessageTranslation = WireMessageTranslation;
/** Whether Translate is worth offering for a message, decided on this device. */
export type BackendMessageTranslationOffer = WireMessageTranslationOffer;
/** One request that left the device, kept for 30 days. */
export type BackendOutboundRequest = WireRequestLogEntry;

export async function backendGetTranslationSettings(): Promise<BackendTranslationSettings> {
  return invoke<BackendTranslationSettings>('get_translation_settings');
}

export async function backendSaveTranslationSettings(
  input: BackendSaveTranslationSettings,
): Promise<BackendTranslationSettings> {
  return invoke<BackendTranslationSettings>('save_translation_settings', { input });
}

/** Stores the key encrypted on this device; `null` removes it. */
export async function backendSetTranslationApiKey(
  engine: BackendTranslationEngineId,
  apiKey: string | null,
): Promise<BackendTranslationSettings> {
  return invoke<BackendTranslationSettings>('set_translation_api_key', { input: { engine, api_key: apiKey } });
}

/** One authenticated request: proves the stored key and reads the allowance. */
export async function backendCheckTranslationEngine(
  engine: BackendTranslationEngineId,
): Promise<BackendTranslationUsage> {
  return invoke<BackendTranslationUsage>('check_translation_engine', { engine });
}

export async function backendListTranslationLanguages(
  engine: BackendTranslationEngineId,
): Promise<BackendTranslationLanguage[]> {
  return invoke<BackendTranslationLanguage[]>('list_translation_languages', { engine });
}

export async function backendSetTranslationConsent(
  engine: BackendTranslationEngineId,
  accountId: string,
  granted: boolean,
): Promise<BackendTranslationSettings> {
  return invoke<BackendTranslationSettings>('set_translation_consent', {
    input: { engine, account_id: accountId, granted },
  });
}

export async function backendMessageTranslationOffer(
  messageId: string,
  targetLanguage: string,
): Promise<BackendMessageTranslationOffer> {
  return invoke<BackendMessageTranslationOffer>('message_translation_offer', {
    input: { message_id: messageId, target_language: targetLanguage },
  });
}

/** The stored translation if there is one; otherwise sends the body to the engine. */
export async function backendTranslateMessage(
  messageId: string,
  targetLanguage: string,
): Promise<BackendMessageTranslation> {
  return invoke<BackendMessageTranslation>('translate_message', {
    input: { message_id: messageId, target_language: targetLanguage },
  });
}

export async function backendListOutboundRequests(): Promise<BackendOutboundRequest[]> {
  return invoke<BackendOutboundRequest[]>('list_outbound_requests');
}

export async function backendClearOutboundRequests(): Promise<void> {
  return invoke<void>('clear_outbound_requests');
}
