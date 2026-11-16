/**
 * The client's i18n instance. Generated catalogues — edit `parts/*.json` and
 * run `pnpm i18n:build`, never `en.json`/`de.json` by hand.
 *
 * Before the first frame only the startup catalogue loads: the keys code in
 * the static import graph of `main.ts` can use. Everything else arrives as a
 * chunk together with the lazy code that needs it (ADR 0017).
 *
 * The viewer and the tray have their own entries (`viewer.ts`, `tray.ts`) with
 * their own subsets; importing this module from shared code would pull the full
 * catalogue into those bundles.
 */
import { createLazyAppI18n } from './create';

const { instance, ready } = createLazyAppI18n({
  en: () => import('./startup.en.json'),
  de: () => import('./startup.de.json'),
});
export const i18n = instance;
export const i18nReady = ready;

export { SUPPORTED_LOCALES, type AppLocale, type LocalePreference } from './locale';
