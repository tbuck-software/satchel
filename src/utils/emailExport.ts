/** Export the whole body, independently of reader modes, quote folding and previews. */
export function emailBodyForExport(html: string | undefined, text: string, format: 'markdown' | 'text'): string {
  if (!html?.trim()) return text;
  const doc = new DOMParser().parseFromString(html, 'text/html');
  doc.querySelectorAll('script, style, head, template').forEach((node) => node.remove());
  const markdown = format === 'markdown';
  const escape = (value: string) => markdown ? value.replace(/[\\`*_[\]<>]/g, '\\$&') : value;
  const children = (node: Node): string => Array.from(node.childNodes).map(render).join('');
  function render(node: Node): string {
    if (node.nodeType === 3) return escape((node.textContent ?? '').replace(/\s+/g, ' '));
    if (node.nodeType !== 1) return '';
    const element = node as Element;
    const tag = element.tagName.toLowerCase();
    if (tag === 'pre') {
      const content = element.textContent ?? '';
      const fence = '`'.repeat(Math.max(3, ...Array.from(content.matchAll(/`+/g), (match) => match[0].length + 1)));
      return markdown ? `\n\n${fence}\n${content}\n${fence}\n\n` : `\n\n${content}\n\n`;
    }
    if (tag === 'code') {
      const content = element.textContent ?? '';
      const fence = '`'.repeat(Math.max(1, ...Array.from(content.matchAll(/`+/g), (match) => match[0].length + 1)));
      return markdown ? `${fence} ${content} ${fence}` : content;
    }
    if (tag === 'br') return '\n';
    if (tag === 'hr') return markdown ? '\n\n---\n\n' : '\n\n';
    // Keep image descriptions without fetching remote images or exporting inline image bytes.
    if (tag === 'img') return escape(element.getAttribute('alt') ?? '');
    const content = children(element);
    if (tag === 'a') {
      const href = element.getAttribute('href') ?? '';
      if (!/^(?:https?:|mailto:|tel:)/i.test(href)) return content;
      const label = content.trim() || escape(href);
      const target = href.replace(/[\s()<>\\]/g, (character) =>
        character === '(' ? '%28' : character === ')' ? '%29' : encodeURIComponent(character));
      return markdown ? `[${label}](${target})` : `${label} (${href})`;
    }
    if (markdown && /^(strong|b|em|i|s|del)$/.test(tag)) {
      const marker = /^(strong|b)$/.test(tag) ? '**' : /^(s|del)$/.test(tag) ? '~~' : '*';
      return content.trim() ? `${marker}${content.trim()}${marker}` : content;
    }
    if (/^h[1-6]$/.test(tag)) return `\n\n${markdown ? '#'.repeat(Number(tag[1])) + ' ' : ''}${content.trim()}\n\n`;
    if (tag === 'blockquote') return `\n\n${content.trim().split('\n').map((line) => markdown ? `> ${line}` : line).join('\n')}\n\n`;
    if (tag === 'li') {
      const index = Array.from(element.parentElement?.children ?? []).indexOf(element);
      const marker = element.parentElement?.tagName === 'OL' ? `${index + 1}.` : '-';
      return `\n${marker} ${content.trim().replace(/\n/g, '\n  ')}\n`;
    }
    if (tag === 'td' || tag === 'th') return `${content.trim()}\t`;
    if (/^(p|div|section|article|header|footer|table|ul|ol|dl|dt|dd)$/.test(tag)) return `\n\n${content.trim()}\n\n`;
    if (tag === 'tr') return `${content.trim()}\n`;
    return content;
  }
  return children(doc.body).replace(/\u00a0/g, ' ').replace(/[ \t]+\n/g, '\n').replace(/\n{3,}/g, '\n\n').trim() || text;
}
