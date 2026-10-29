import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import { router } from './router';
import { i18n, i18nReady } from '@/i18n';
import './styles/fonts.css';
import './styles/tokens.css';
import './styles/tailwind.css';
import './styles/base.css';
import './styles/transitions.css';
import { installInteractionDiagnostics } from '@/utils/diagnostics';
import { installGlobalErrorHandlers } from '@/utils/globalErrors';
import { renderUnsupportedRuntimeNotice, supportsRequiredCss } from '@/utils/webviewSupport';
import {
  backendRuntimeCapabilities,
  backendSupportsNativeCommands,
} from '@/services/backend';
import {
  applyRuntimeShellMetadata,
  runtimePlatformFromUserAgent,
  runtimeShellMetadata,
  type RuntimePlatform,
} from '@/utils/runtimeShell';
import { readTheme } from '@/persistence/theme';
import { installKeyboardInset } from '@/mobile/keyboardInset';

// Before mount: the settings store lands on the same value later, so a light
// user never sees a dark first frame.
document.documentElement.dataset.theme = readTheme();

let oauthBridgeInstalled = false;

/**
 * Take the OAuth deep link before anything can start a flow.
 *
 * A phone finishes its login through the registered scheme, and the callback
 * may arrive into a cold start, so this cannot wait for the account stores.
 * Desktop stays out: it answers its own loopback callback in Rust.
 */
function installMobileOAuthBridge(platform: RuntimePlatform) {
  if (oauthBridgeInstalled) return;
  if (platform !== 'android' && platform !== 'ios') return;
  oauthBridgeInstalled = true;
  void import('@/utils/mobileOAuth')
    .then(({ installOAuthCallbackBridge }) => installOAuthCallbackBridge())
    .catch((error) => {
      console.error('OAuth deep-link handler could not be installed', error);
    });
}

function configureRuntimeShell() {
  const nativeRuntime = backendSupportsNativeCommands();
  const fallbackPlatform: RuntimePlatform = nativeRuntime
    ? runtimePlatformFromUserAgent(navigator.userAgent, navigator.maxTouchPoints)
    : 'web';

  // Apply a safe platform class before mounting, but do not hold first paint
  // behind an IPC round trip. The native capability result corrects the UA
  // fallback asynchronously if the embedded WebView identifies itself oddly.
  applyRuntimeShellMetadata(
    document.documentElement,
    runtimeShellMetadata(fallbackPlatform, nativeRuntime),
  );
  if (!nativeRuntime) return;
  installMobileOAuthBridge(fallbackPlatform);

  void backendRuntimeCapabilities()
    .then(({ platform }) => {
      if (platform !== fallbackPlatform) {
        applyRuntimeShellMetadata(
          document.documentElement,
          runtimeShellMetadata(platform, nativeRuntime),
        );
        installMobileOAuthBridge(platform);
      }
    })
    .catch((error) => {
      console.warn('runtime platform detection failed', error);
    });
}

function mountApp() {
  // Checked before mounting: on a runtime without color-mix the app comes up
  // looking blank rather than broken, and the user has no way to tell why.
  const root = document.querySelector<HTMLElement>('#app');
  if (root && !supportsRequiredCss()) {
    renderUnsupportedRuntimeNotice(root);
    return;
  }

  configureRuntimeShell();
  installKeyboardInset();
  installInteractionDiagnostics();
  const app = createApp(App).use(createPinia()).use(i18n).use(router);
  installGlobalErrorHandlers(app, {
    openDiagnostics: () => router.push({ name: 'settings', query: { section: 'diagnostics' } }),
  });
  void router.isReady().then(() => app.mount('#app'));
}

void i18nReady.then(mountApp);
