/**
 * Are we running inside a Tauri webview?
 *
 * This answers "may I touch windows, events, the tray, the filesystem" — not
 * "may I issue commands". The two differ in exactly one build: `dev:mock`
 * serves a real browser with a stand-in command transport (`./mock.ts`).
 * Use `backendCommandsAvailable()` for anything that goes through `invoke`.
 */
export function backendSupportsNativeCommands(): boolean {
  if (typeof window === 'undefined') {
    return false;
  }

  const tauriWindow = window as Window & {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
    __TAURI_METADATA__?: unknown;
  };

  if (tauriWindow.__TAURI_INTERNALS__ || tauriWindow.__TAURI__ || tauriWindow.__TAURI_METADATA__) {
    return true;
  }

  return typeof navigator !== 'undefined' && navigator.userAgent.includes('Tauri');
}
