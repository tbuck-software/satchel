import { reactive } from 'vue';
import { imagePolicyPersistence } from '@/persistence/imagePolicy';
import type { ImagePolicyMode } from '@/types/settings';
import { normalizeDomain, senderDomain } from '@/utils/senderDomains';

/**
 * When remote images load without an explicit grant:
 * - `allowlist`: never — only the allow-list below counts.
 * - `accepted`: for senders the user accepted in the new-sender review;
 *   the caller supplies that check, this module does not read the store.
 * - `all`: always.
 */
export type { ImagePolicyMode } from '@/types/settings';

// Module-level by design: the renderer and Settings share one policy snapshot,
// hydrated synchronously when this client entry imports the composable.
const state = reactive({
  allowedSenders: imagePolicyPersistence.allowedSenders.read(),
  allowedDomains: imagePolicyPersistence.allowedDomains.read(),
  mode: imagePolicyPersistence.mode.read(),
});

export function useImagePolicy() {
  function isAllowed(senderEmail?: string | null, trustedDmarcPass = false): boolean {
    if (state.mode === 'all') return true;
    if (!senderEmail) return false;
    const email = senderEmail.trim().toLowerCase();
    if (state.allowedSenders.has(email)) return true;
    const domain = senderDomain(email);
    return trustedDmarcPass && domain !== '' && state.allowedDomains.has(domain);
  }

  function allow(senderEmail: string) {
    state.allowedSenders.add(senderEmail.trim().toLowerCase());
    imagePolicyPersistence.allowedSenders.write(state.allowedSenders);
  }

  function revoke(senderEmail: string) {
    state.allowedSenders.delete(senderEmail.trim().toLowerCase());
    imagePolicyPersistence.allowedSenders.write(state.allowedSenders);
  }

  function allowDomain(domain: string) {
    const normalized = normalizeDomain(domain);
    if (!normalized) return;
    state.allowedDomains.add(normalized);
    imagePolicyPersistence.allowedDomains.write(state.allowedDomains);
  }

  function revokeDomain(domain: string) {
    state.allowedDomains.delete(normalizeDomain(domain));
    imagePolicyPersistence.allowedDomains.write(state.allowedDomains);
  }

  function setMode(mode: ImagePolicyMode) {
    state.mode = mode;
    imagePolicyPersistence.mode.write(mode);
  }

  return { state, isAllowed, allow, revoke, allowDomain, revokeDomain, setMode };
}

function isLocalResourceUrl(url: string): boolean {
  const lower = url.trim().toLowerCase();
  return lower.startsWith('data:')
    || lower.startsWith('cid:')
    || lower.startsWith('blob:')
    || lower.startsWith('asset:')
    || lower.startsWith('http://asset.localhost/')
    || lower.startsWith('https://asset.localhost/');
}

/**
 * Rewrite remote resource references (`img src`, `img srcset`, the legacy
 * `background` attribute, `background: url(...)` in style attrs) into inert
 * data-blocked-* placeholders so no network request fires. Inline data:/cid:/
 * blob: references are kept (these aren't tracking pixels).
 *
 * The renderer uses an isolated mail document, so this function is also the
 * enforcement layer for stylesheet URLs and @import rules.
 */
export function blockRemoteImages(html: string): { html: string; blockedCount: number } {
  let blockedCount = 0;
  const next = html
    .replace(
      /<img\b([^>]*?)\ssrc\s*=\s*(['"])([^'"]+)\2/gi,
      (full, attrs: string, quote: string, url: string) => {
        if (isLocalResourceUrl(url)) {
          return full;
        }
        blockedCount += 1;
        return `<img${attrs} data-blocked-src=${quote}${url}${quote}`;
      },
    )
    // srcset is allowed through the sanitizer and loads independently of src.
    // Note: srcset can't be split on commas — data: URLs contain commas. Local
    // iff nothing remains after removing local URLs and width/density descriptors.
    .replace(
      /<img\b([^>]*?)\ssrcset\s*=\s*(['"])([^'"]+)\2/gi,
      (full, attrs: string, quote: string, value: string) => {
        const remainder = value
          .replace(/data:\S+/gi, '')
          .replace(/(?:cid|blob):[^\s,]+/gi, '')
          .replace(/\b\d+(?:\.\d+)?[wx]\b/gi, '')
          .replace(/[,\s]+/g, '');
        if (!remainder) {
          return full;
        }
        blockedCount += 1;
        return `<img${attrs} data-blocked-srcset=${quote}${value}${quote}`;
      },
    )
    // Legacy HTML background attribute (<table background="https://...">).
    .replace(
      /\sbackground\s*=\s*(['"])([^'"]+)\1/gi,
      (full, quote: string, url: string) => {
        if (isLocalResourceUrl(url)) {
          return full;
        }
        blockedCount += 1;
        return ` data-blocked-background=${quote}${url}${quote}`;
      },
    );
  // Also strip background-image: url(...) on style attrs to avoid pixel loaders.
  const stripped = next.replace(
    /style\s*=\s*(['"])([^'"]*background[^'"]*url\([^'"]*?\)[^'"]*?)\1/gi,
    (_full, quote: string, body: string) => {
      const cleaned = body.replace(/background[^;]*url\([^)]*\)\s*;?/gi, '');
      return `style=${quote}${cleaned}${quote}`;
    },
  );
  // `(?:https?:)?\/\/` also catches protocol-relative URLs (`url(//host/p.gif)`),
  // which resolve against the document base and would otherwise slip through.
  const remoteImportPattern = /@import\s+(?:url\()?\s*['"]?(?:https?:)?\/\/[^;)]*(?:\))?\s*;?/gi;
  const remoteUrlPattern = /url\(\s*['"]?(?:https?:)?\/\/[^)]*\)/gi;
  const withoutRemoteStyles = stripped.replace(
    /<style\b([^>]*)>([\s\S]*?)<\/style>/gi,
    (_full, attrs: string, css: string) => {
      const imports = css.match(remoteImportPattern)?.length ?? 0;
      const withoutImports = css.replace(remoteImportPattern, '');
      const urls = withoutImports.match(remoteUrlPattern)?.length ?? 0;
      blockedCount += imports + urls;
      const cleaned = withoutImports.replace(remoteUrlPattern, 'none');
      return `<style${attrs}>${cleaned}</style>`;
    },
  );
  return { html: withoutRemoteStyles, blockedCount };
}
