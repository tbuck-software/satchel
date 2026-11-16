// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { blockRemoteImages, blockRemoteImagesInDocument } from './useImagePolicy';

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

  it('keeps the shape a blocked picture was laid out with', () => {
    const { html } = blockRemoteImages(
      '<img src="https://img.example/k.jpg" width="268" height="160" alt="Coffee" style="display:block">'
      + '<img src="https://img.example/p.jpg" width=300 height=200>',
    );
    const [coffee, product] = Array.from(new DOMParser().parseFromString(html, 'text/html').images);
    expect(coffee?.getAttribute('data-blocked-src')).toBe('https://img.example/k.jpg');
    expect(coffee?.hasAttribute('data-blocked-shape')).toBe(true);
    expect(coffee?.getAttribute('style')).toBe('aspect-ratio:268/160;display:block');
    expect(product?.hasAttribute('data-blocked-shape')).toBe(true);
    expect(product?.getAttribute('style')).toBe('aspect-ratio:300/200;');
  });

  it('leaves a tracking pixel and an unsized image as they are', () => {
    const pixel = blockRemoteImages('<img src="https://t.example/p.gif" width="1" height="1">').html;
    const unsized = blockRemoteImages('<img src="https://img.example/a.jpg" style="max-width:200px">').html;
    expect(pixel).not.toContain('aspect-ratio');
    expect(pixel).not.toContain('data-blocked-shape');
    expect(unsized).not.toContain('aspect-ratio');
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
      '<table><tr><td style="background: url(https://t.example/x.png); color: red">x</td></tr></table>',
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

  it('blocks a source whose URL carries the other quote character', () => {
    const { html, blockedCount } = blockRemoteImages(`<img src="https://t.example/p?a'b" alt='say "hi"'>`);
    expect(blockedCount).toBe(1);
    const image = new DOMParser().parseFromString(html, 'text/html').images[0];
    expect(image?.hasAttribute('src')).toBe(false);
    expect(image?.getAttribute('data-blocked-src')).toBe("https://t.example/p?a'b");
  });

  it('blocks a source that follows an attribute containing >', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<img alt="a > b" src="https://t.example/p.gif"><table title=">" background="https://t.example/bg.png"></table>',
    );
    expect(blockedCount).toBe(2);
    const doc = new DOMParser().parseFromString(html, 'text/html');
    expect(doc.querySelector('[src], [background]')).toBeNull();
  });

  it('blocks unquoted and entity-encoded sources', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<img src=https://t.example/a.gif><img src="&#104;ttps://t.example/b.gif">',
    );
    expect(blockedCount).toBe(2);
    expect(new DOMParser().parseFromString(html, 'text/html').querySelector('[src]')).toBeNull();
  });

  it('blocks url() spelled with CSS escapes', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<style>.a { background: \\75 rl(https://t.example/a.png) } @\\69mport "https://t.example/b.css";</style>'
      + '<p style="background-image: u\\72l(https://t.example/c.png)">x</p>',
    );
    expect(blockedCount).toBe(3);
    expect(html).not.toContain('t.example');
  });

  it('blocks remote strings in image-set(), image() and src()', () => {
    const { html, blockedCount } = blockRemoteImages(
      '<style>.a { background: image-set("https://t.example/a.png" 1x, \'data:image/png;base64,AAA\' 2x) }'
      + ' .b { background: -webkit-image-set(\'//t.example/b.png\' 1x) } .c { background: src("https://t.example/c.png") }</style>'
      + '<p style="background: image(\'https://t.example/d.png\')">x</p>',
    );
    expect(blockedCount).toBe(4);
    expect(html).not.toContain('t.example');
  });

  it('keeps local references in image-set() and url()', () => {
    const source = '<style>.a { background: image-set("data:image/png;base64,AAA" 1x, "cid:logo@x" 2x) }'
      + ' .b { fill: url(#gradient) }</style><p style="background: url(cid:bg@x)">x</p>';
    const { html, blockedCount } = blockRemoteImages(source);
    expect(blockedCount).toBe(0);
    expect(html).toBe(source);
  });

  it('cannot close a style element by cutting an import out of it', () => {
    const doc = new DOMParser().parseFromString(
      '<style></sty@import;le><link rel="preconnect" href="https://t.example"></style><p>x</p>',
      'text/html',
    );
    blockRemoteImagesInDocument(doc);
    const reparsed = new DOMParser().parseFromString(doc.head.innerHTML + doc.body.innerHTML, 'text/html');
    expect(reparsed.querySelector('link')).toBeNull();
  });

  it('does not re-block already blocked markup', () => {
    const first = blockRemoteImages('<img src="https://t.example/p.gif">');
    const second = blockRemoteImages(first.html);
    expect(second.blockedCount).toBe(0);
    expect(second.html).toBe(first.html);
  });
});
