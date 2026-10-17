const SKIPPED_ELEMENTS = new Set(['style', 'script', 'template', 'noscript', 'head', 'title']);

type StyleSource = (element: Element) => CSSStyleDeclaration | null;

function inlineStyle(element: Element): CSSStyleDeclaration | null {
  return (element as HTMLElement).style ?? null;
}

/** Nearest declared value of an inherited property; inline styles only say where they were set. */
function inherited(chain: Element[], styleOf: StyleSource, read: (style: CSSStyleDeclaration) => string): string {
  for (const element of chain) {
    const style = styleOf(element);
    const value = style ? read(style) : '';
    if (value) return value;
  }
  return '';
}

function textIsHidden(chain: Element[], styleOf: StyleSource): boolean {
  for (const element of chain) {
    if (element.hasAttribute('hidden') || SKIPPED_ELEMENTS.has(element.localName)) return true;
    const style = styleOf(element);
    if (style && (style.display === 'none' || style.opacity === '0')) return true;
  }
  const visibility = inherited(chain, styleOf, (style) => style.visibility);
  if (visibility === 'hidden' || visibility === 'collapse') return true;
  const fontSize = inherited(chain, styleOf, (style) => style.fontSize);
  return fontSize !== '' && Number.parseFloat(fontSize) === 0;
}

/**
 * The text a reader sees in a link, for comparing it with the destination.
 *
 * `textContent` also carries hidden spans, so `paypal.com<span hidden>!</span>`
 * never read as a hostname. A rendered document is asked for computed styles;
 * a parsed one only has inline styles to go by.
 */
export function visibleLinkText(anchor: Element): string {
  const doc = anchor.ownerDocument;
  const view = doc.defaultView;
  const styleOf: StyleSource = view && anchor.isConnected
    ? (element) => view.getComputedStyle(element)
    : inlineStyle;
  const walker = doc.createTreeWalker(anchor, 0x1 | 0x4 /* SHOW_ELEMENT | SHOW_TEXT */);
  let text = '';
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeType === 1) {
      if ((node as Element).localName === 'br') text += ' ';
      continue;
    }
    const chain: Element[] = [];
    for (let element = node.parentElement; element; element = element.parentElement) {
      chain.push(element);
      if (element === anchor) break;
    }
    if (!textIsHidden(chain, styleOf)) text += node.nodeValue ?? '';
  }
  return text.replace(/\s+/g, ' ').trim();
}
