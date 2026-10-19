import { createApp } from 'vue';
import { createPinia } from 'pinia';
import MessageFileViewer from './MessageFileViewer.vue';
import { viewerI18n, viewerI18nReady } from '@/i18n/viewer';
import { readTheme } from '@/persistence/theme';
import { loadPublicSuffixTable } from '@/utils/externalLinks';
import '@/styles/fonts.css';
import '@/styles/tokens.css';
import '@/styles/tailwind.css';
import '@/styles/base.css';
import '@/styles/transitions.css';

// No router, no sync bootstrap, no account loading: this window exists to put
// one message on screen. Pinia is here only because the shared renderer reads
// the settings store, which stays at its defaults. The theme comes straight
// from storage instead — `persistence/theme` pulls no store in behind it.
document.documentElement.dataset.theme = readTheme();
// The message is this window's only purpose and its links are assessed on the
// first render, so the public-suffix table loads alongside it rather than on
// demand.
void loadPublicSuffixTable();

void viewerI18nReady.then(() => createApp(MessageFileViewer).use(createPinia()).use(viewerI18n).mount('#app'));
