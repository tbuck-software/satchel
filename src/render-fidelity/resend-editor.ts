import { createApp, h } from 'vue';
import { createPinia } from 'pinia';
import { i18n, i18nReady } from '../i18n';
import RichTextEditor from '../components/mail/RichTextEditor.vue';
import { resendEditorHtml } from '../utils/composerThreadPrefill';
import '../styles/tokens.css';

declare global {
  interface Window {
    __mountSatchelResendEditor?: (sourceHtml: string) => void;
  }
}

// Module imports and catalogues finish before Playwright can request a mount.
// Keep imports in Vite's graph, including Vue's versioned dependency URL.
await i18nReady;
window.__mountSatchelResendEditor = (sourceHtml) => {
  const html = resendEditorHtml(sourceHtml);
  createApp({ render: () => h(RichTextEditor, { modelValue: '', html }) })
    .use(createPinia()).use(i18n).mount('#editor');
};
