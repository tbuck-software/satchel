// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { emailBodyForExport } from './emailExport';
import { marked } from 'marked';

describe('complete email body export', () => {
  it('retains all paragraphs, quoted history and link targets from an HTML-only mail', () => {
    const html = '<style>p { color: red }</style><h1>Welcome</h1><p>First paragraph</p>'
      + '<p><a href="https://example.test/login?q=(value)&amp;next=%2Finbox">Sign in</a></p>'
      + '<blockquote><p>Earlier message</p><p>Earlier footer</p></blockquote><p>Final footer</p>';
    const result = emailBodyForExport(html, '', 'markdown');
    const rendered = new DOMParser().parseFromString(marked.parse(result) as string, 'text/html');
    expect(rendered.querySelector('h1')?.textContent).toBe('Welcome');
    expect(rendered.querySelector('a')?.getAttribute('href')).toBe('https://example.test/login?q=%28value%29&next=%2Finbox');
    expect(rendered.querySelector('blockquote')?.textContent).toContain('Earlier footer');
    expect(rendered.body.textContent).toContain('Final footer');
    expect(result).not.toContain('color: red');
  });

  it('keeps literal Markdown punctuation and code readable without exporting executable content', () => {
    const result = emailBodyForExport('<p>[literal] *text*</p><pre>line 1\n  line 2</pre>'
      + '<p><a href="javascript:alert(1)">Unsafe target</a><img src="https://example.test/pixel" alt="Diagram"></p>', '', 'markdown');
    const rendered = new DOMParser().parseFromString(marked.parse(result) as string, 'text/html');
    expect(rendered.querySelector('p')?.textContent).toBe('[literal] *text*');
    expect(rendered.querySelector('pre')?.textContent).toContain('line 1\n  line 2');
    expect(rendered.querySelectorAll('a, img, script')).toHaveLength(0);
    expect(rendered.body.textContent).toContain('Diagram');
    expect(result).not.toContain('javascript:');
  });

  it('keeps an entire plain-text body byte for byte, including its quoted history', () => {
    const text = 'Hi\n\n' + 'Long body. '.repeat(500) + '\n> Earlier message\nFinal footer';
    expect(emailBodyForExport(undefined, text, 'markdown')).toBe(text);
    expect(emailBodyForExport('', text, 'text')).toBe(text);
  });
});
