import { createApp } from 'vue';
import { createPinia } from 'pinia';
import TrayPopover from './TrayPopover.vue';
import { trayI18n, trayI18nReady } from '@/i18n/tray';
import { readTheme } from '@/persistence/theme';
import '@/styles/fonts.css';
import '@/styles/tokens.css';
import '@/styles/tailwind.css';
import '@/styles/base.css';
import '@/styles/transitions.css';
import './tray.css';

document.documentElement.dataset.theme = readTheme();

void trayI18nReady.then(() => createApp(TrayPopover).use(createPinia()).use(trayI18n).mount('#app'));
