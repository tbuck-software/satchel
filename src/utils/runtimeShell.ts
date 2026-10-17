import type { BackendRuntimeCapabilities } from '@/services/backend';

export type RuntimePlatform = BackendRuntimeCapabilities['platform'] | 'web';

export interface RuntimeShellMetadata {
  platform: RuntimePlatform;
  mobile: boolean;
  windowChrome: boolean;
}

/**
 * Window chrome is a property of the native host, not of the viewport width.
 * A narrow macOS window still has traffic lights; a wide phone or tablet does
 * not. Keeping that distinction outside the responsive store prevents the two
 * layout concerns from drifting back together.
 */
export function runtimeShellMetadata(
  platform: RuntimePlatform,
  nativeRuntime: boolean,
): RuntimeShellMetadata {
  const mobile = platform === 'android' || platform === 'ios';
  return {
    platform,
    mobile,
    windowChrome: nativeRuntime && !mobile,
  };
}

/**
 * Whether the host is a phone or tablet shell, as opposed to a narrow window.
 *
 * Reads the attribute `main.ts` writes from `runtime_capabilities`, so callers
 * outside the bootstrap do not have to await the capability query.
 */
export function isMobileRuntime(): boolean {
  if (typeof document === 'undefined') return false;
  return document.documentElement.dataset.runtimeMobile === 'true';
}

export function applyRuntimeShellMetadata(
  root: HTMLElement,
  metadata: RuntimeShellMetadata,
): void {
  root.dataset.runtimePlatform = metadata.platform;
  root.dataset.runtimeMobile = String(metadata.mobile);
  root.dataset.windowChrome = String(metadata.windowChrome);
}

export function runtimePlatformFromUserAgent(
  userAgent: string,
  maxTouchPoints = 0,
): Exclude<RuntimePlatform, 'web'> {
  if (/android/i.test(userAgent)) return 'android';
  if (/(iphone|ipad|ipod)/i.test(userAgent)) return 'ios';
  // iPadOS may identify itself as macOS when desktop-class browsing is active.
  if (/macintosh/i.test(userAgent) && maxTouchPoints > 1) return 'ios';
  if (/(macintosh|mac os x)/i.test(userAgent)) return 'macos';
  if (/windows/i.test(userAgent)) return 'windows';
  return 'linux';
}
