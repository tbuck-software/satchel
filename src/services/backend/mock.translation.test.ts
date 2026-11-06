// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  BackendMessageTranslation,
  BackendMessageTranslationOffer,
  BackendOutboundRequest,
  BackendTranslationSettings,
  BackendTranslationUsage,
} from './translation';

// Mirrors `commands/translation.rs`: the order of checks, the key that never
// comes back, and a deterministic "engine" the browser suite can recognise.

type Send = <T>(command: string, payload?: Record<string, unknown>) => Promise<T>;

async function mockBackend(): Promise<Send> {
  const { sendCommand } = await import('./mock');
  return sendCommand;
}

async function firstMessage(): Promise<{ id: string; accountId: string }> {
  const { mockMessagesByThread, mockThreads } = await import('@/data/mock');
  const thread = mockThreads.find((entry) => (mockMessagesByThread[entry.id] ?? []).some((m) => m.bodyHtml || m.bodyText))!;
  const message = mockMessagesByThread[thread.id]!.find((m) => m.bodyHtml || m.bodyText)!;
  return { id: message.id, accountId: thread.accountId };
}

describe('mock translation', () => {
  beforeEach(() => { vi.resetModules(); });

  it('stores the key without ever returning it', async () => {
    const send = await mockBackend();
    const saved = await send<BackendTranslationSettings>('set_translation_api_key', {
      input: { engine: 'deepl', api_key: ' secret-key:fx ' },
    });
    expect(JSON.stringify(saved)).not.toContain('secret');
    expect(saved.engines[0]).toMatchObject({ id: 'deepl', has_key: true, plan: 'free', local: false });
    await expect(send('set_translation_api_key', { input: { engine: 'deepl', api_key: 'two words' } }))
      .rejects.toThrow('backend.errors.translation.keyMalformed');
    const usage = await send<BackendTranslationUsage>('check_translation_engine', { engine: 'deepl' });
    expect(usage).toMatchObject({ limit: 500_000, unit: 'characters' });
  });

  it('refuses without consent, then translates once and serves the cache', async () => {
    const send = await mockBackend();
    const { id, accountId } = await firstMessage();
    await send('set_translation_api_key', { input: { engine: 'deepl', api_key: 'k:fx' } });
    await send('save_translation_settings', { input: { engine: 'deepl', target_language: null, log_sent_text: false } });
    const input = { message_id: id, target_language: 'FR' };

    await expect(send('translate_message', { input })).rejects.toThrow('backend.errors.translation.consentRequired');
    expect((await send<BackendMessageTranslationOffer>('message_translation_offer', { input })).consented).toBe(false);

    await send('set_translation_consent', { input: { engine: 'deepl', account_id: accountId, granted: true } });
    const first = await send<BackendMessageTranslation>('translate_message', { input });
    expect(first.cached).toBe(false);
    expect(first.body_html ?? first.body_text).toContain('[FR]');
    const again = await send<BackendMessageTranslation>('translate_message', { input });
    expect(again.cached).toBe(true);

    const log = await send<BackendOutboundRequest[]>('list_outbound_requests');
    expect(log).toHaveLength(1);
    expect(log[0]).toMatchObject({ feature: 'translation', provider: 'deepl', outcome: 'ok', sent_text: null });
    await send('clear_outbound_requests');
    expect(await send<BackendOutboundRequest[]>('list_outbound_requests')).toEqual([]);
  });

  it('says so when no engine is set up', async () => {
    const send = await mockBackend();
    const { id } = await firstMessage();
    await expect(send('translate_message', { input: { message_id: id, target_language: 'DE' } }))
      .rejects.toThrow('backend.errors.translation.notConfigured');
  });
});
