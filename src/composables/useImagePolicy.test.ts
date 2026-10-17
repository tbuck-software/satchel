import { describe, expect, it } from 'vitest';
import { blockRemoteImages } from './useImagePolicy';

function stillLoadsRemote(html: string): boolean {
  return (
    /(?<!data-blocked-)(?:src|srcset|background)\s*=\s*['"]https?:/i.test(html) ||
    /style\s*=\s*"[^"]*url\(https?:/i.test(html)
  );
}

describe('blockRemoteImages', () => {
  it('blocks remote img src', () => {
    const { html, blockedCount } = blockRemoteImages('<img src="https://t.example/p.gif">');
    expect(blockedCount).toBe(1);
    expect(stillLoadsRemote(html)).toBe(false);
    expect(html).toContain('data-blocked-src="https://t.example/p.gif"');
  });

  it('keeps data:, cid: and blob: sources', () => {
    for (const url of ['data:image/png;base64,AAA', 'cid:logo@x', 'blob:abc']) {
      const { html, blockedCount } = blockRemoteImages(`<img src="${url}">`);
      expect(blockedCount).toBe(0);
      expect(html).toContain(`src="${url}"`);
    }
  });

  it('blocks remote srcset (sanitizer allows the attribute)', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<img srcset="https://t.example/a.png 1x, https://t.example/b.png 2x">',
    );
    expect(blockedCount).toBe(1);
    expect(stillLoadsRemote(html)).toBe(false);
    expect(html).toContain('data-blocked-srcset=');
  });

  it('keeps local-only srcset', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<img srcset="data:image/png;base64,AAA 1x">',
    );
    expect(blockedCount).toBe(0);
    expect(html).toContain('srcset="data:image/png;base64,AAA 1x"');
  });

  it('blocks the legacy background attribute', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<table background="https://t.example/bg.png"><tr><td>x</td></tr></table>',
    );
    expect(blockedCount).toBe(1);
    expect(stillLoadsRemote(html)).toBe(false);
    expect(html).toContain('data-blocked-background=');
  });

  it('strips background url() from style attributes', () => {
    const { html } = blockRemoteImages(
      '<td style="background: url(https://t.example/x.png); color: red">x</td>',
    );
    expect(stillLoadsRemote(html)).toBe(false);
    expect(html).toContain('color: red');
  });

  it('blocks remote URLs and imports inside style blocks', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<style>@import url("https://t.example/mail.css"); .hero { background:url(https://t.example/x.png) }</style><p>Mail</p>',
    );

    expect(blockedCount).toBe(2);
    expect(html).not.toContain('https://t.example');
    expect(html).toContain('<p>Mail</p>');
  });

  it('counts src and srcset on the same tag separately', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<img src="https://a.example/x" srcset="https://a.example/y 1x">',
    );
    expect(blockedCount).toBe(2);
    expect(stillLoadsRemote(html)).toBe(false);
  });

  it('blocks protocol-relative URLs inside style blocks', () => {
    // `url(//host/x.png)` resolves against the document base and used to slip
    // past the https?-anchored patterns entirely.
    const { html, blockedCount } = blockRemoteImages(
      '<style>@import url("//t.example/mail.css"); .hero { background:url(//t.example/x.png) }</style>',
    );

    expect(blockedCount).toBe(2);
    expect(html).not.toContain('t.example');
  });

  it('blocks protocol-relative img sources', () => {
    const { html, blockedCount } = blockRemoteImages('<img src="//t.example/p.gif">');
    expect(blockedCount).toBe(1);
    expect(html).toContain('data-blocked-src="//t.example/p.gif"');
  });

  it('does not re-block already blocked markup', () => {
    const first = blockRemoteImages('<img src="https://t.example/p.gif">');
    const second = blockRemoteImages(first.html);
    expect(second.blockedCount).toBe(0);
    expect(second.html).toBe(first.html);
  });
});
