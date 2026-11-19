import { reactive } from 'vue';
import { imagePolicyPersistence } from '@/persistence/imagePolicy';
import type { ImagePolicyMode } from '@/types/settings';
import { isLocalResourceUrl, neutralizeRemoteCss } from '@/utils/remoteCss';
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
  /**
   * Allowed domains that also count for mail without a DMARC pass. Many
   * servers write no Authentication-Results Satchel can trust (it trusts only
   * receivers whose header boundary it knows), so a domain allow would never
   * apply there. Whoever forges such a sender gets the images too; the user
   * opts in per domain, knowing that.
   */
  unverifiedDomains: imagePolicyPersistence.unverifiedDomains.read(),
  mode: imagePolicyPersistence.mode.read(),
});

export function useImagePolicy() {
  function isAllowed(senderEmail?: string | null, trustedDmarcPass = false): boolean {
    if (state.mode === 'all') return true;
    if (!senderEmail) return false;
    const email = senderEmail.trim().toLowerCase();
    if (state.allowedSenders.has(email)) return true;
    const domain = senderDomain(email);
    if (domain === '' || !state.allowedDomains.has(domain)) return false;
    return trustedDmarcPass || state.unverifiedDomains.has(domain);
  }

  function allow(senderEmail: string) {
    state.allowedSenders.add(senderEmail.trim().toLowerCase());
    imagePolicyPersistence.allowedSenders.write(state.allowedSenders);
  }

  function revoke(senderEmail: string) {
    state.allowedSenders.delete(senderEmail.trim().toLowerCase());
    imagePolicyPersistence.allowedSenders.write(state.allowedSenders);
  }

  /**
   * `includeUnverified` also loads images for mail from the domain that did
   * not pass DMARC. Without it, an earlier opt-in for the domain stays.
   */
  function allowDomain(domain: string, options: { includeUnverified?: boolean } = {}) {
    const normalized = normalizeDomain(domain);
    if (!normalized) return;
    state.allowedDomains.add(normalized);
    imagePolicyPersistence.allowedDomains.write(state.allowedDomains);
    if (options.includeUnverified) setDomainUnverified(normalized, true);
  }

  /** Whether mail from an allowed domain loads images without a DMARC pass. */
  function setDomainUnverified(domain: string, allowed: boolean) {
    const normalized = normalizeDomain(domain);
    if (!normalized) return;
    if (allowed) state.unverifiedDomains.add(normalized);
    else state.unverifiedDomains.delete(normalized);
    imagePolicyPersistence.unverifiedDomains.write(state.unverifiedDomains);
  }

  function revokeDomain(domain: string) {
    const normalized = normalizeDomain(domain);
    state.allowedDomains.delete(normalized);
    imagePolicyPersistence.allowedDomains.write(state.allowedDomains);
    if (state.unverifiedDomains.delete(normalized)) {
      imagePolicyPersistence.unverifiedDomains.write(state.unverifiedDomains);
    }
  }

  function setMode(mode: ImagePolicyMode) {
    state.mode = mode;
    imagePolicyPersistence.mode.write(mode);
  }

  return { state, isAllowed, allow, revoke, allowDomain, setDomainUnverified, revokeDomain, setMode };
}

const MIN_PLACEHOLDER_SIDE = 24;
const PIXEL_SIZE = /^\s*(\d+(?:\.\d+)?)(?:px)?\s*$/i;
const DOCUMENT_TAG = /<(?:!doctype|html|head|body)[\s>/]/i;

function pixelSize(value: string | null): number {
  return Number(PIXEL_SIZE.exec(value ?? '')?.[1]);
}

/**
 * A blocked image keeps the box its width and height attributes promise.
 *
 * The mail document scales every image with `height: auto`, which a picture
 * that never loads has nothing to scale from: it collapsed to its alt text,
 * and two products side by side got boxes of different heights. An
 * `aspect-ratio` from the attributes holds the shape the sender laid out.
 */
function keepBlockedImageShape(image: Element) {
  const width = pixelSize(image.getAttribute('width'));
  const height = pixelSize(image.getAttribute('height'));
  // A tracking pixel stays a pixel; only a picture gets a placeholder.
  if (!(width >= MIN_PLACEHOLDER_SIDE && height >= MIN_PLACEHOLDER_SIDE)) return;
  if (image.hasAttribute('data-blocked-shape')) return;
  image.setAttribute('data-blocked-shape', '');
  image.setAttribute('style', `aspect-ratio:${width}/${height};${image.getAttribute('style') ?? ''}`);
}

function isLocalSrcset(value: string): boolean {
  // srcset can't be split on commas: data: URLs contain commas. Local iff
  // nothing remains after removing local URLs and width/density descriptors.
  const remainder = value
    .replace(/data:\S+/gi, '')
    .replace(/(?:cid|blob):[^\s,]+/gi, '')
    .replace(/\b\d+(?:\.\d+)?[wx]\b/gi, '')
    .replace(/[,\s]+/g, '');
  return !remainder;
}

function blockAttribute(element: Element, name: string, isLocal: (value: string) => boolean): boolean {
  const value = element.getAttribute(name);
  if (value === null || !value.trim() || isLocal(value)) return false;
  element.removeAttribute(name);
  element.setAttribute(`data-blocked-${name}`, value);
  return true;
}

/**
 * Rewrite remote resource references (`img src`, `img srcset`, the legacy
 * `background` attribute, `url()` and friends in style attributes and style
 * blocks) into inert data-blocked-* placeholders so no network request fires.
 * Inline data:/cid:/blob: references are kept (these aren't tracking pixels).
 *
 * Works on a parsed, inert document, so attribute values are read the way the
 * renderer reads them, whatever quotes or `>` they contain. Returns how many
 * references it blocked.
 */
export function blockRemoteImagesInDocument(doc: Document): number {
  let blockedCount = 0;
  for (const image of Array.from(doc.querySelectorAll('img'))) {
    const src = blockAttribute(image, 'src', isLocalResourceUrl);
    // srcset loads independently of src.
    const srcset = blockAttribute(image, 'srcset', isLocalSrcset);
    blockedCount += Number(src) + Number(srcset);
    if (src || srcset) keepBlockedImageShape(image);
  }
  // Legacy HTML background attribute (<table background="https://...">).
  for (const element of Array.from(doc.querySelectorAll('[background]'))) {
    if (blockAttribute(element, 'background', isLocalResourceUrl)) blockedCount += 1;
  }
  for (const element of Array.from(doc.querySelectorAll('[style]'))) {
    const style = element.getAttribute('style') ?? '';
    const cleaned = neutralizeRemoteCss(style);
    if (cleaned.css !== style) element.setAttribute('style', cleaned.css);
    blockedCount += cleaned.blocked;
  }
  for (const style of Array.from(doc.querySelectorAll('style'))) {
    const css = style.textContent ?? '';
    const cleaned = neutralizeRemoteCss(css);
    // Cutting `@import;` out of `</sty@import;le>` must not close the element
    // once it is serialized; `\/` is the same character to CSS.
    if (cleaned.css !== css) style.textContent = cleaned.css.replace(/<\/(style)/gi, '<\\/$1');
    blockedCount += cleaned.blocked;
  }
  return blockedCount;
}

/** `blockRemoteImagesInDocument` for markup, returned as the parser read it. */
export function blockRemoteImages(html: string): { html: string; blockedCount: number } {
  if (!html) return { html, blockedCount: 0 };
  const doc = new DOMParser().parseFromString(html, 'text/html');
  const blockedCount = blockRemoteImagesInDocument(doc);
  const serialized = DOCUMENT_TAG.test(html)
    ? doc.documentElement.outerHTML
    : `${doc.head.innerHTML}${doc.body.innerHTML}`;
  return { html: serialized, blockedCount };
}
