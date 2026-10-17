import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  installOAuthCallbackBridge,
  isAllowedOAuthAuthorizationUrl,
  parseOAuthCallbackUrl,
  processOAuthCallbackUrls,
} from './mobileOAuth';

const { backendAbortOauthFlow, backendCompleteOauthFlow, getCurrent, onOpenUrl } = vi.hoisted(
  () => ({
    backendAbortOauthFlow: vi.fn(),
    backendCompleteOauthFlow: vi.fn(),
    getCurrent: vi.fn(),
    onOpenUrl: vi.fn(),
  }),
);

vi.mock('@/services/backend', () => ({ backendAbortOauthFlow, backendCompleteOauthFlow }));
vi.mock('@tauri-apps/plugin-deep-link', () => ({ getCurrent, onOpenUrl }));

describe('mobile OAuth callback parsing', () => {
  it('accepts the registered callback and extracts state and code', () => {
    expect(
      parseOAuthCallbackUrl('de.tbuck.satchel://oauth/callback?state=st-123&code=a%2Fb'),
    ).toEqual({ kind: 'code', state: 'st-123', code: 'a/b' });
  });

  it.each([
    'https://attacker.invalid/oauth/callback?state=st-123&code=x',
    'de.tbuck.satchel://thread/123?state=st-123&code=x',
    'de.tbuck.satchel://oauth/callback?code=x',
    'de.tbuck.satchel://oauth/callback?state=st-123',
  ])('rejects an invalid callback: %s', (url) => {
    expect(parseOAuthCallbackUrl(url)).toBeNull();
  });

  it('reports provider cancellation against the flow it ends', () => {
    expect(
      parseOAuthCallbackUrl(
        'de.tbuck.satchel://oauth/callback?state=st-123&error=access_denied&error_description=Cancelled',
      ),
    ).toEqual({ kind: 'refused', state: 'st-123', reason: 'Cancelled' });
  });


  it('reports asynchronous callback completion failures instead of leaking rejections', async () => {
    const error = new Error('state expired');
    const onError = vi.fn();

    await processOAuthCallbackUrls(
      ['de.tbuck.satchel://oauth/callback?state=st-123&code=code'],
      async () => Promise.reject(error),
      onError,
    );

    expect(onError).toHaveBeenCalledWith(error);
  });
});

describe('mobile OAuth authorization URLs', () => {
  it('opens the two providers it knows, over TLS only', () => {
    expect(isAllowedOAuthAuthorizationUrl('https://accounts.google.com/o/oauth2/v2/auth?x=1')).toBe(
      true,
    );
    expect(
      isAllowedOAuthAuthorizationUrl('https://login.microsoftonline.com/common/oauth2/v2.0/authorize'),
    ).toBe(true);
  });

  it.each([
    'http://accounts.google.com/o/oauth2/v2/auth',
    'https://accounts.google.com.attacker.invalid/o/oauth2/v2/auth',
    'https://attacker.invalid/o/oauth2/v2/auth',
    'javascript:alert(1)',
    'de.tbuck.satchel://oauth/callback',
    'not a url',
  ])('refuses %s', (url) => {
    expect(isAllowedOAuthAuthorizationUrl(url)).toBe(false);
  });
});

describe('mobile OAuth callback bridge', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getCurrent.mockResolvedValue(null);
    onOpenUrl.mockResolvedValue(() => {});
  });

  async function deliver(url: string): Promise<unknown[]> {
    const errors: unknown[] = [];
    const originalError = console.error;
    console.error = (...args: unknown[]) => errors.push(args[1]);
    try {
      await installOAuthCallbackBridge();
      const [handler] = onOpenUrl.mock.calls[0] as [(urls: string[]) => void];
      getCurrent.mockResolvedValue([url]);
      handler([url]);
      // The listener fires and forgets; let its promise chain settle.
      await vi.waitFor(() => expect(onOpenUrl).toHaveBeenCalled());
      await vi.waitFor(() => expect(backendCompleteOauthFlow.mock.calls.length + backendAbortOauthFlow.mock.calls.length).toBe(1));
    } finally {
      console.error = originalError;
    }
    return errors;
  }

  it('completes the flow in the backend exactly once per callback', async () => {
    backendCompleteOauthFlow.mockResolvedValue({});

    await deliver('de.tbuck.satchel://oauth/callback?state=st-1&code=code-1');

    expect(backendCompleteOauthFlow).toHaveBeenCalledTimes(1);
    expect(backendCompleteOauthFlow).toHaveBeenCalledWith({ state: 'st-1', code: 'code-1' });
  });

  it('ends the flow instead of spending a code when the provider refuses', async () => {
    backendAbortOauthFlow.mockResolvedValue(undefined);
    const warn = console.warn;
    console.warn = () => {};
    try {
      await deliver(
        'de.tbuck.satchel://oauth/callback?state=st-1&error=access_denied&error_description=Cancelled',
      );
    } finally {
      console.warn = warn;
    }

    expect(backendCompleteOauthFlow).not.toHaveBeenCalled();
    // Without this the caller watches a spinner until the flow times out.
    expect(backendAbortOauthFlow).toHaveBeenCalledWith('st-1');
  });
});

describe('mobile OAuth startup wiring', () => {
  /**
   * The callback can arrive into a cold start, so the handler has to be in
   * place before any screen could have started a flow.
   */
  it('installs the callback bridge from the application entry point', () => {
    const entry = readFileSync(new URL('../main.ts', import.meta.url), 'utf8');

    expect(entry).toContain("import { installOAuthCallbackBridge } from '@/utils/mobileOAuth'");
    expect(entry).toContain('installOAuthCallbackBridge()');
    expect(entry).toMatch(/installMobileOAuthBridge\(fallbackPlatform\)/);
  });
});

describe('mobile OAuth authorization hand-off', () => {
  /**
   * `webbrowser::open` has no desktop session to hand a URL to on a phone, so
   * the backend reports that it opened nothing and the shell has to take over.
   * A caller that ignores the answer leaves the user watching a spinner until
   * the flow times out.
   */
  it('lets the shell open what the backend could not, in every caller', () => {
    const root = fileURLToPath(new URL('..', import.meta.url));
    const callers: string[] = [];
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full);
        else if (/\.(ts|vue)$/.test(entry.name) && !entry.name.includes('.test.')) {
          const source = readFileSync(full, 'utf8');
          if (full.includes(join('services', 'backend'))) continue;
          if (source.includes('backendStartOauthFlow(')) callers.push(full);
        }
      }
    };
    walk(root);

    expect(callers.length).toBeGreaterThan(0);
    for (const caller of callers) {
      const source = readFileSync(caller, 'utf8');
      expect(source, caller).toContain('browser_opened');
      // Whoever waits for the flow to finish has to make sure it can start.
      if (source.includes('backendPollOauthFlow')) {
        expect(source, caller).toContain('openOAuthAuthorizationUrl');
      }
    }
  });
});
