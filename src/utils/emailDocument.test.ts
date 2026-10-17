// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { buildIsolatedDocument, htmlToPlainText } from './emailDocument';

function parse(html: string): Document {
  return new DOMParser().parseFromString(html, 'text/html');
}

function anchor(html: string, autoDark = false): HTMLAnchorElement {
  const doc = parse(buildIsolatedDocument(html, autoDark));
  const element = doc.querySelector('a');
  if (!element) throw new Error('no anchor in output');
  return element as HTMLAnchorElement;
}

describe('buildIsolatedDocument', () => {
  it('wraps the body in the styling hook', () => {
    const doc = parse(buildIsolatedDocument('<p>Hallo</p>', false));
    const wrapper = doc.querySelector('.satchel-email-body');
    expect(wrapper).not.toBeNull();
    expect(wrapper!.innerHTML).toContain('<p>Hallo</p>');
  });

  it('carries the original body class and style onto the wrapper', () => {
    const output = buildIsolatedDocument(
      '<html><body class="mail-body" style="margin:0"><p>x</p></body></html>',
      false,
    );
    const wrapper = parse(output).querySelector<HTMLElement>('.satchel-email-body')!;
    expect(wrapper.classList.contains('mail-body')).toBe(true);
    expect(wrapper.getAttribute('style')).toContain('margin:0');
  });

  it('restores sanitized source-body presentation onto the wrapper', () => {
    const output = buildIsolatedDocument(
      '<div data-satchel-source-body="on" class="campaign" style="font-family:Arial,sans-serif"></div><h1>Briefing</h1>',
      false,
    );
    const wrapper = parse(output).querySelector<HTMLElement>('.satchel-email-body')!;
    expect(wrapper.classList.contains('campaign')).toBe(true);
    expect(wrapper.style.fontFamily).toBe('Arial, sans-serif');
    expect(wrapper.querySelector('[data-satchel-source-body]')).toBeNull();
    expect(wrapper.querySelector('h1')?.textContent).toBe('Briefing');
  });

  it('applies auto-dark to restored source-body colours', () => {
    const output = buildIsolatedDocument(
      '<div data-satchel-source-body="on" style="background:#fff;font-family:Arial,sans-serif"></div><p>x</p>',
      true,
    );
    const wrapper = parse(output).querySelector<HTMLElement>('.satchel-email-body')!;
    expect(wrapper.style.background).toBe('var(--satchel-email-canvas)');
    expect(wrapper.style.fontFamily).toBe('Arial, sans-serif');
  });

  it('keeps head styles ahead of the wrapper', () => {
    const output = buildIsolatedDocument(
      '<html><head><style>.a{color:red}</style></head><body><p>x</p></body></html>',
      false,
    );
    expect(output.indexOf('<style>')).toBeLessThan(output.indexOf('satchel-email-body'));
  });

  it('marks undesigned mail so it can be styled as simple text', () => {
    const wrapper = parse(buildIsolatedDocument('<p>Kurze Notiz</p>', false)).querySelector(
      '.satchel-email-body',
    )!;
    expect(wrapper.getAttribute('data-satchel-simple')).toBe('on');
  });

  it('flags the auto-dark pass on the wrapper', () => {
    const wrapper = parse(buildIsolatedDocument('<p>x</p>', true)).querySelector(
      '.satchel-email-body',
    )!;
    expect(wrapper.getAttribute('data-satchel-auto-dark')).toBe('on');
  });

  it('leaves colours untouched when auto-dark is off', () => {
    const output = buildIsolatedDocument('<p style="color:#000000">x</p>', false);
    expect(output).toContain('#000000');
  });

  it('adapts legacy table canvases without replacing an explicit content fill', () => {
    const html = '<table bgcolor="#eeeeee"><tr><td bgcolor="white">Mail</td><td bgcolor="white" style="background:#174c70">Banner</td></tr></table>';
    const adapted = parse(buildIsolatedDocument(html, true));
    expect(adapted.querySelector('table')!.style.backgroundColor).toBe('var(--satchel-email-canvas)');
    const cells = adapted.querySelectorAll('td');
    expect(cells[0]!.style.backgroundColor).toBe('var(--satchel-email-canvas)');
    expect(cells[1]!.style.backgroundColor).toBe('rgb(23, 76, 112)');

    const original = parse(buildIsolatedDocument(html, false));
    expect(original.querySelector('table')!.getAttribute('bgcolor')).toBe('#eeeeee');
    expect(original.querySelector('td')!.getAttribute('bgcolor')).toBe('white');
  });

  /**
   * Mail is full of tables pinned to a desktop width. Left alone they force the
   * reading pane to scroll sideways.
   */
  it('makes a fixed-width table responsive', () => {
    const table = parse(
      buildIsolatedDocument('<table width="800"><tr><td>x</td></tr></table>', false),
    ).querySelector('table')!;
    expect(table.style.getPropertyValue('max-width')).toBe('100%');
    expect(table.style.getPropertyPriority('width')).toBe('important');
  });

  describe('link handling', () => {
    it('routes a plain link outward with the anchor hardened', () => {
      const link = anchor('<a href="https://example.com/x">Beispiel</a>');
      expect(link.getAttribute('target')).toBe('_blank');
      expect(link.getAttribute('rel')).toContain('noopener');
      expect(link.getAttribute('rel')).toContain('noreferrer');
    });

    /**
     * The security-relevant case. A link the assessment rejects must lose its
     * href outright — an anchor without one cannot be followed by any means,
     * where a merely styled-as-disabled link still can.
     */
    it('strips the href from a rejected link', () => {
      const link = anchor('<a href="javascript:alert(1)">klick</a>');
      expect(link.hasAttribute('href')).toBe(false);
      expect(link.getAttribute('aria-disabled')).toBe('true');
    });

    // #125: the label used to be `textContent`, so a hidden "!" turned the host into prose.
    it('judges the label a reader sees, not the hidden text behind it', () => {
      const link = anchor(
        '<a href="https://evil.example/login">paypal.com<span style="display:none">!</span></a>',
      );
      expect(link.getAttribute('data-satchel-link-risk')).toBe('true');
    });

    it('marks a link that needs confirmation so the click handler can stop', () => {
      const link = anchor('<a href="https://phish.example/login">https://bank.example</a>');
      expect(link.getAttribute('data-satchel-link-risk')).toBe('true');
      expect(link.getAttribute('href')).toBe('https://phish.example/login');
    });

    it('replaces a known Google redirect with its assessed destination', () => {
      const link = anchor(
        '<a href="https://www.google.com/url?q=https%3A%2F%2Fchat.example%2Froom&amp;source=calendar">https://chat.example/room</a>',
      );
      expect(link.getAttribute('href')).toBe('https://chat.example/room');
      expect(link.hasAttribute('data-satchel-link-risk')).toBe(false);
    });

    it('keeps mailto links and clears any stale risk marker', () => {
      const link = anchor(
        '<a href="mailto:lena@example.com" data-satchel-link-risk="true" aria-disabled="true">schreiben</a>',
      );
      expect(link.getAttribute('href')).toContain('mailto:lena@example.com');
      expect(link.hasAttribute('data-satchel-link-risk')).toBe(false);
      expect(link.hasAttribute('aria-disabled')).toBe(false);
    });

    it('does not duplicate rel values that were already present', () => {
      const link = anchor('<a href="https://example.com/x" rel="noopener">x</a>');
      const rel = (link.getAttribute('rel') ?? '').split(/\s+/).filter(Boolean);
      expect(new Set(rel).size).toBe(rel.length);
    });

    it('leaves anchors without an href alone', () => {
      const output = buildIsolatedDocument('<a name="top">x</a>', false);
      expect(output).toContain('name="top"');
    });
  });

  it('produces no markup for an empty body', () => {
    const wrapper = parse(buildIsolatedDocument('', false)).querySelector('.satchel-email-body')!;
    expect(wrapper.innerHTML.trim()).toBe('');
  });
});

describe('htmlToPlainText', () => {
  it('renders blocks and breaks as line breaks', () => {
    expect(htmlToPlainText('<h1>Titel</h1><p>Absatz<br>zweite Zeile</p>')).toBe(
      'Titel\nAbsatz\nzweite Zeile',
    );
  });

  it('drops script and style content', () => {
    const output = htmlToPlainText('<style>.a{color:red}</style><script>x=1</script><p>Text</p>');
    expect(output).toBe('Text');
  });

  it('collapses runs of blank lines', () => {
    expect(htmlToPlainText('<p>a</p><p></p><p></p><p></p><p>b</p>')).toBe('a\n\nb');
  });

  it('normalises non-breaking spaces', () => {
    expect(htmlToPlainText('<p>a&nbsp;b</p>')).toBe('a b');
  });

  it('returns nothing for empty markup', () => {
    expect(htmlToPlainText('  ')).toBe('');
  });
});
