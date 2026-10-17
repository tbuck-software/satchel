import { createApp } from 'vue';
import { createPinia } from 'pinia';
import RenderFidelityApp from './RenderFidelityApp.vue';
import { viewerI18n } from '@/i18n/viewer';
import '@/styles/tokens.css';
import '@/styles/tailwind.css';
import '@/styles/base.css';
import '@/styles/transitions.css';

document.documentElement.dataset.theme = 'dark';

createApp(RenderFidelityApp).use(createPinia()).use(viewerI18n).mount('#app');
