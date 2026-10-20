import {
  darkModeBackgroundColor,
  isDesignedEmailMarkup,
  readableDarkModeTextColor,
  responsiveEmailTableWidth,
  rewriteCssColorsForDark,
} from '@/utils/emailAppearance';
import { assessExternalLink, parseMailtoLink } from '@/utils/externalLinks';
import { visibleLinkText } from '@/utils/linkLabel';

/**
 * Prepare sanitized mail HTML for display inside the isolated document.
 *
 * The security boundary is elsewhere — `ammonia` on the backend, the CSP from
 * `emailCsp.ts`, and the script-less iframe sandbox. What happens here is the
 * presentation layer on top of that: adapting colours to a dark theme, keeping
 * fixed-width layouts from overflowing, and routing links through the
 * confirmation flow.
 *
 * The link pass is the one part with a safety role. It runs after
 * sanitisation and marks each anchor by verdict, so the click handler in the
 * component never has to re-derive whether a destination was trusted.
 *
 * `resolveInlineImages` swaps `cid:` references for loadable sources in the
 * finished tree, right before serialization, so the inline images never pass
 * through the passes above as text.
 */
export function buildIsolatedDocument(
  html: string,
  autoDark: boolean,
  resolveInlineImages?: (roots: Element[]) => void,
): string {
  const parser = new DOMParser();
  const doc = parser.parseFromString(html, 'text/html');
  doc.querySelectorAll('link, meta').forEach((node) => node.remove());

  // The backend sanitizer works on fragments, whose parser drops attributes
  // from a document-level body. It prepends a sanitized, empty marker with the
  // safe presentation attributes instead. Only the first element can be that
  // trusted marker; sender markup follows it after sanitisation.
  const firstElement = doc.body?.firstElementChild;
  const sourceBodyMarker = firstElement?.getAttribute('data-satchel-source-body') === 'on'
    ? firstElement as HTMLElement
    : null;
  const sourceBody = sourceBodyMarker ?? doc.body;
  if (autoDark && sourceBodyMarker?.hasAttribute('style')) {
    sourceBodyMarker.setAttribute(
      'style',
      rewriteCssColorsForDark(sourceBodyMarker.getAttribute('style') ?? ''),
    );
  }
  sourceBodyMarker?.remove();

  const designedLayout = isDesignedEmailMarkup(doc.body?.innerHTML ?? html);

  if (autoDark) {
    for (const style of Array.from(doc.querySelectorAll('style'))) {
      style.textContent = rewriteCssColorsForDark(style.textContent ?? '');
    }
    for (const element of Array.from(doc.querySelectorAll<HTMLElement>('[style], [color], [bgcolor]'))) {
      const inlineStyle = element.getAttribute('style');
      if (inlineStyle) {
        element.setAttribute('style', rewriteCssColorsForDark(inlineStyle));
      }
      const color = element.getAttribute('color');
      const mappedColor = color ? readableDarkModeTextColor(color) : null;
      if (mappedColor) element.setAttribute('color', mappedColor);
      const background = element.getAttribute('bgcolor');
      const mappedBackground = background ? darkModeBackgroundColor(background) : null;
      if (mappedBackground) {
        // Legacy bgcolor attributes cannot resolve custom properties.
        if (!element.style.background && !element.style.backgroundColor) {
          element.style.backgroundColor = mappedBackground;
        }
        element.removeAttribute('bgcolor');
      }
    }
  }

  // Mail is full of tables pinned to a desktop width. Left alone they force the
  // reading pane to scroll sideways.
  for (const table of Array.from(doc.querySelectorAll<HTMLTableElement>('table'))) {
    const responsiveWidth = responsiveEmailTableWidth(
      table.getAttribute('width'),
      table.style.width,
    );
    if (!responsiveWidth) continue;
    table.style.setProperty('width', responsiveWidth, 'important');
    table.style.setProperty('max-width', '100%', 'important');
  }

  for (const link of Array.from(doc.querySelectorAll('a[href]'))) {
    const href = link.getAttribute('href') ?? '';
    const label = visibleLinkText(link);
    const mailto = parseMailtoLink(href);
    if (mailto) {
      link.setAttribute('href', mailto.href);
      link.setAttribute('target', '_blank');
      link.setAttribute('rel', 'noopener noreferrer');
      link.removeAttribute('aria-disabled');
      link.removeAttribute('data-satchel-link-risk');
      continue;
    }
    const assessment = assessExternalLink(href, label);
    if (!assessment.allowed) {
      // Drop the href entirely rather than styling it as disabled: an anchor
      // without one cannot be followed by any means.
      link.setAttribute('aria-disabled', 'true');
      link.removeAttribute('href');
      continue;
    }
    link.setAttribute('href', assessment.href);
    if (assessment.requiresConfirmation) {
      link.setAttribute('data-satchel-link-risk', 'true');
    }
    link.setAttribute('target', '_blank');
    const rel = new Set((link.getAttribute('rel') || '').split(/\s+/).filter(Boolean));
    rel.add('noopener');
    rel.add('noreferrer');
    link.setAttribute('rel', Array.from(rel).join(' '));
  }

  const wrapper = doc.createElement('div');
  wrapper.className = 'satchel-email-body';
  if (!designedLayout) {
    wrapper.setAttribute('data-satchel-simple', 'on');
  }
  if (autoDark) {
    wrapper.setAttribute('data-satchel-auto-dark', 'on');
  }
  // Carry the original <body> class and style onto the wrapper: mail styles
  // routinely target the body, and the body itself does not survive the move
  // into the isolated document.
  const bodyClass = sourceBody?.getAttribute('class');
  if (bodyClass) {
    wrapper.classList.add(...bodyClass.split(/\s+/).filter(Boolean));
  }
  const bodyStyle = sourceBody?.getAttribute('style');
  if (bodyStyle) {
    wrapper.setAttribute('style', bodyStyle);
  }
  if (autoDark) {
    wrapper.style.setProperty('background', 'var(--satchel-email-canvas)', 'important');
  }
  wrapper.innerHTML = doc.body?.innerHTML ?? html;

  const headStyles = Array.from(doc.head?.querySelectorAll('style') ?? []);
  // Last, so the colour and table passes above never walk image bytes.
  resolveInlineImages?.([wrapper, ...headStyles]);
  const styles = headStyles
    .map((style) => style.outerHTML)
    .join('');
  return `${styles}${wrapper.outerHTML}`;
}

/**
 * Readable text for a message that only shipped HTML.
 *
 * Separate from `textFromHtml` in `emailQuotes.ts`, which serves comparison
 * rather than reading: this one drops non-content elements and treats headings
 * as blocks, where the other covers the block elements that carry quote
 * structure. Changing either to match the other would shift quote hashes.
 */
export function htmlToPlainText(html: string): string {
  if (!html.trim()) return '';
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('script, style, head').forEach((node) => node.remove());
  doc.querySelectorAll('br').forEach((node) => node.replaceWith('\n'));
  doc.querySelectorAll('p, div, tr, li, h1, h2, h3, h4, h5, h6').forEach((node) => {
    node.append('\n');
  });
  return (doc.body?.textContent ?? '')
    .replace(/\u00a0/g, ' ')
    .replace(/[ \t]+\n/g, '\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}
