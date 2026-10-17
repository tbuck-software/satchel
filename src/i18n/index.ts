/**
 * The client's i18n instance. Generated catalogues — edit `parts/*.json` and
 * run `pnpm i18n:build`, never `en.json`/`de.json` by hand.
 *
 * The viewer and the tray have their own entries (`viewer.ts`, `tray.ts`) with
 * their own subsets; importing this module from shared code would pull the full
 * catalogue into those bundles.
 */
import { createLazyAppI18n } from './create';

const { instance, ready } = createLazyAppI18n({
  en: () => import('./en.json'),
  de: () => import('./de.json'),
});
export const i18n = instance;
export const i18nReady = ready;

export { SUPPORTED_LOCALES, type AppLocale, type LocalePreference } from './locale';
