import { describe, expect, it, vi } from 'vitest';
import { createI18n } from 'vue-i18n';
import de from '@/i18n/de.json';
import { setActiveI18n, t } from '@/i18n/runtime';
import { backendErrorKey, translateBackendError } from '@/utils/backendErrors';

/*
 * The frontend used to throw "Desktop backend unavailable. Start Satchel with
 * `npm run tauri:dev`." — an English sentence, naming a command this repo does
 * not use, that around 75 call sites interpolate into an already German toast
 * (`t('thread.move.failed', { error: error.message })`). Rust has answered with
 * `{ key, params }` since day one; this checks that this side does too.
 */

vi.mock('@/services/backend/transport', () => ({
  backendCommandsAvailable: () => false,
  sendCommand: vi.fn(),
}));

const { invoke, normalizeInvokeError } = await import('@/services/backend/core');

const i18n = createI18n({ legacy: false, locale: 'de', messages: { de } });
setActiveI18n(i18n.global as never);

describe('backend errors carry a key, not a sentence', () => {
  it('throws a translatable error when there is no desktop backend', async () => {
    const error = await invoke('list_threads').catch((thrown: unknown) => thrown);

    expect(backendErrorKey(error)).toBe('backend.errors.desktopUnavailable');
    expect(translateBackendError(error)).toBe(t('backend.errors.desktopUnavailable'));
  });

  it('says it in the reader\'s language, not in English', async () => {
    const error = (await invoke('list_threads').catch((thrown: unknown) => thrown)) as Error;

    expect(error.message).toBe(t('backend.errors.desktopUnavailable'));
    expect(error.message).not.toContain('Desktop backend unavailable');
    // The command in the sentence has to be one this repo actually has.
    expect(error.message).not.toContain('npm run tauri:dev');
  });

  it('turns a Tauri runtime failure into the same key', () => {
    const normalized = normalizeInvokeError(new Error('not running inside Tauri runtime'));

    expect(backendErrorKey(normalized)).toBe('backend.errors.desktopUnavailable');
  });
});
