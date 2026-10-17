import { describe, expect, it } from 'vitest';
import {
  applyRuntimeShellMetadata,
  runtimePlatformFromUserAgent,
  runtimeShellMetadata,
} from './runtimeShell';

describe('runtime shell metadata', () => {
  it.each(['macos', 'windows', 'linux'] as const)(
    'keeps native %s window chrome in compact layouts',
    (platform) => {
      expect(runtimeShellMetadata(platform, true)).toEqual({
        platform,
        mobile: false,
        windowChrome: true,
      });
    },
  );

  it.each(['android', 'ios'] as const)(
    'uses system safe areas without desktop chrome on %s',
    (platform) => {
      expect(runtimeShellMetadata(platform, true)).toEqual({
        platform,
        mobile: true,
        windowChrome: false,
      });
    },
  );

  it('does not render native titlebar space in a browser preview', () => {
    expect(runtimeShellMetadata('web', false).windowChrome).toBe(false);
  });

  it('writes CSS-addressable metadata before the app mounts', () => {
    const root = { dataset: {} } as HTMLElement;
    applyRuntimeShellMetadata(root, runtimeShellMetadata('ios', true));
    expect(root.dataset).toMatchObject({
      runtimePlatform: 'ios',
      runtimeMobile: 'true',
      windowChrome: 'false',
    });
  });

  it.each([
    ['Mozilla/5.0 (Linux; Android 15)', 'android'],
    ['Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)', 'ios'],
    ['Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7)', 'macos'],
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64)', 'windows'],
    ['Tauri/2.0 (X11; Linux x86_64)', 'linux'],
  ] as const)('uses %s as a native-only fallback', (userAgent, platform) => {
    expect(runtimePlatformFromUserAgent(userAgent)).toBe(platform);
  });

  it('recognizes an iPad using its desktop-class macOS user agent', () => {
    expect(runtimePlatformFromUserAgent('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15)', 5))
      .toBe('ios');
  });
});
