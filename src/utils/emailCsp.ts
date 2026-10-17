const LOCAL_RESOURCE_SOURCES = 'data: cid: blob: asset: http://asset.localhost https://asset.localhost';

/**
 * Content-Security-Policy for the isolated mail document.
 *
 * This is the actual enforcement of the remote-content policy: it stops fetches
 * that the regex blocker in `useImagePolicy` cannot reach — srcset variants,
 * `url()` inside <style> blocks, `@import`, protocol-relative URLs and the
 * legacy `background` attribute.
 *
 * The backend sanitizer strips <meta>, so mail content cannot ship a competing
 * policy, and the meta is emitted first in <head> so it covers everything that
 * follows.
 *
 * Scripts stay impossible three times over: sanitizer + iframe sandbox without
 * `allow-scripts` + `default-src 'none'`.
 *
 * Regression guard: `src/utils/emailCsp.test.ts`. This policy was silently lost
 * once during a renderer refactor (commit aa03d78), which left the tracking
 * protection resting on the regex layer alone. Do not remove it without
 * replacing the enforcement.
 */
export function buildEmailCsp(allowRemoteImages: boolean): string {
  const remote = allowRemoteImages ? ' https: http:' : '';
  return [
    "default-src 'none'",
    `img-src ${LOCAL_RESOURCE_SOURCES}${remote}`,
    "style-src 'unsafe-inline'",
    `font-src data:${remote}`,
    `media-src ${LOCAL_RESOURCE_SOURCES}${remote}`,
    "object-src 'none'",
    "form-action 'none'",
    "base-uri 'none'",
  ].join('; ');
}
