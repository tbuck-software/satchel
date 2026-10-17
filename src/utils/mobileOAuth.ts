import { verifiedDeepLinkSource } from './openUrl';
import { backendAbortOauthFlow, backendCompleteOauthFlow } from '@/services/backend';
import { BackendError } from '@/utils/backendErrors';

/**
 * What the registered scheme delivered for one flow.
 *
 * A refusal is an outcome, not a parse failure: it names the flow it ends, and
 * the backend has to hear about it or the flow keeps looking pending.
 */
export type OAuthCallback =
  | { kind: 'code'; state: string; code: string }
  | { kind: 'refused'; state: string; reason: string };

const CALLBACK_SCHEME = 'de.tbuck.satchel:';
const CALLBACK_HOST = 'oauth';
const CALLBACK_PATH = '/callback';
const AUTHORIZATION_HOSTS = new Set([
  'accounts.google.com',
  'login.microsoftonline.com',
]);

export function parseOAuthCallbackUrl(rawUrl: string): OAuthCallback | null {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return null;
  }

  if (
    url.protocol !== CALLBACK_SCHEME ||
    url.hostname !== CALLBACK_HOST ||
    url.pathname !== CALLBACK_PATH
  ) {
    return null;
  }

  const state = url.searchParams.get('state')?.trim();
  if (!state) return null;

  const error = url.searchParams.get('error')?.trim();
  if (error) {
    const description = url.searchParams.get('error_description')?.trim();
    return { kind: 'refused', state, reason: description || error };
  }

  const code = url.searchParams.get('code')?.trim();
  if (!code) return null;
  return { kind: 'code', state, code };
}

export function isAllowedOAuthAuthorizationUrl(rawUrl: string): boolean {
  try {
    const url = new URL(rawUrl);
    return url.protocol === 'https:' && AUTHORIZATION_HOSTS.has(url.hostname);
  } catch {
    return false;
  }
}

export async function openOAuthAuthorizationUrl(rawUrl: string): Promise<void> {
  if (!isAllowedOAuthAuthorizationUrl(rawUrl)) {
    throw new BackendError('backend.errors.oauthUrlRejected');
  }
  const { openUrl } = await import('@tauri-apps/plugin-opener');
  await openUrl(rawUrl);
}

export async function processOAuthCallbackUrls(
  urls: string[],
  handler: (callback: OAuthCallback) => Promise<void>,
  onError: (error: unknown) => void,
): Promise<void> {
  for (const rawUrl of urls) {
    try {
      const callback = parseOAuthCallbackUrl(rawUrl);
      if (callback) await handler(callback);
    } catch (error) {
      onError(error);
    }
  }
}

export async function installOAuthDeepLinkHandler(
  handler: (callback: OAuthCallback) => Promise<void>,
  onError: (error: unknown) => void = (error) => {
    console.error('Failed to complete OAuth callback', error);
  },
): Promise<() => void> {
  const source = verifiedDeepLinkSource(await import('@tauri-apps/plugin-deep-link'));
  const { getCurrent, onOpenUrl } = source;

  const processUrls = (urls: string[]) => {
    void processOAuthCallbackUrls(urls, handler, onError);
  };

  const unlisten = await onOpenUrl(processUrls);
  const current = await getCurrent();
  if (current) await processOAuthCallbackUrls(current, handler, onError);
  return unlisten;
}

/**
 * Hand every OAuth callback the system delivers to the backend.
 *
 * On mobile this is the only caller of `complete_oauth_flow`: there is no
 * loopback listener, the authorization happens in another app, and the code
 * arrives as a deep link — possibly into a cold start. Install it at startup,
 * before any flow can be started, not from the screen that started one.
 *
 * Failures reach the user through the flow's own polling, because
 * `complete_oauth_flow` and `abort_oauth_flow` both record them against the
 * flow state.
 */
export function installOAuthCallbackBridge(): Promise<() => void> {
  return installOAuthDeepLinkHandler(async (callback) => {
    if (callback.kind === 'code') {
      await backendCompleteOauthFlow({ state: callback.state, code: callback.code });
      return;
    }
    console.warn('OAuth authorization was refused', callback.reason);
    await backendAbortOauthFlow(callback.state);
  });
}
