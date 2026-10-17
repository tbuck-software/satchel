import { t } from '@/i18n/runtime';

/**
 * The one CSS feature the whole design system rests on.
 *
 * Every colour token is mixed at runtime — 260-odd `color-mix()` calls across
 * the components. Where it is missing the browser drops the entire declaration
 * it appears in, so a button keeps its white label and loses its blue
 * background: invisible, not merely unstyled. That failure mode is worse than
 * refusing to start, because it looks like the app is broken rather than the
 * runtime being too old.
 *
 * `color-mix()` shipped in Chrome 111 (March 2023). Android is where this bites:
 * the system WebView updates through the Play Store, so most devices are far
 * ahead of that, but an emulator image or a phone cut off from Play can sit on
 * something much older.
 */
export function supportsRequiredCss(): boolean {
  if (typeof CSS === 'undefined' || typeof CSS.supports !== 'function') return false;
  return CSS.supports('color', 'color-mix(in srgb, red 50%, blue)');
}

/**
 * Replace the page with a plain-HTML explanation.
 *
 * Deliberately styled inline and without a single token: the reason we are here
 * is that the token system does not render.
 */
export function renderUnsupportedRuntimeNotice(target: HTMLElement): void {
  target.innerHTML = `
    <div style="
      max-width: 34rem; margin: 0 auto; padding: 2.5rem 1.5rem;
      font: 16px/1.6 system-ui, sans-serif; color: #111;
    ">
      <h1 style="font-size: 1.4rem; margin: 0 0 1rem;">${t('errors.unsupportedRuntime.title')}</h1>
      <p style="margin: 0 0 1rem;">${t('errors.unsupportedRuntime.body')}</p>
      <p style="margin: 0;">${t('errors.unsupportedRuntime.android')}</p>
    </div>
  `;
}
