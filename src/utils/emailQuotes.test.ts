// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  buildQuotePart,
  dedupeQuoteParts,
  extractHtmlBody,
  extractPlainTextBody,
  isSignatureLikeFragment,
  mergeQuoteContinuations,
  normalizeForHash,
  normalizePlainTextIndent,
  quoteIntroFromText,
  quoteLabel,
  splitSignatureIntroFragment,
  stableHash,
  stripQuoteMarkers,
  textFromHtml,
  type QuotePart,
} from './emailQuotes';

function part(text: string, index = 0): QuotePart {
  const built = buildQuotePart({ text }, index);
  if (!built) throw new Error(`fragment too short to be a quote: ${text}`);
  return built;
}

describe('normalizePlainTextIndent', () => {
  /**
   * The threshold protects deliberately indented content. Code blocks and
   * ASCII tables must survive; only mail that is drowning in indentation is
   * reflowed.
   */
  it('leaves lightly indented text exactly as it was', () => {
    const code = ['function f() {', '            return 1;', '}'].join('\n');
    expect(normalizePlainTextIndent(code)).toBe(code);
  });

  it('flattens indentation once enough lines are affected', () => {
    const input = Array.from({ length: 8 }, (_, i) => `${' '.repeat(16)}line ${i}`).join('\n');
    const output = normalizePlainTextIndent(input);
    expect(output.split('\n').every((line) => line.startsWith('    line'))).toBe(true);
  });

  /**
   * The depth cap rides on the same threshold as the reflow, and quote markers
   * do not count towards it — only whitespace indent does. Deeply nested quotes
   * in an otherwise unindented message therefore keep every level.
   */
  it('leaves quote depth alone while the message is below the threshold', () => {
    const deep = Array.from({ length: 8 }, (_, i) => `${'> '.repeat(6)}line ${i}`).join('\n');
    expect(normalizePlainTextIndent(deep)).toBe(deep);
  });

  it('caps quote depth at three levels once the message is being reflowed', () => {
    const deep = Array.from(
      { length: 8 },
      (_, i) => `${' '.repeat(16)}${'> '.repeat(6)}line ${i}`,
    ).join('\n');
    expect(normalizePlainTextIndent(deep).split('\n')[0]).toBe('> > > line 0');
  });

  it('normalises CRLF', () => {
    const input = Array.from({ length: 8 }, (_, i) => `${' '.repeat(16)}x${i}`).join('\r\n');
    expect(normalizePlainTextIndent(input)).not.toContain('\r');
  });
});

describe('stableHash', () => {
  it('is deterministic', () => {
    expect(stableHash('hallo')).toBe(stableHash('hallo'));
  });

  it('separates different inputs', () => {
    expect(stableHash('hallo')).not.toBe(stableHash('hallo!'));
  });

  it('is always eight hex characters', () => {
    for (const input of ['', 'a', 'a longer piece of quoted text']) {
      expect(stableHash(input)).toMatch(/^[0-9a-f]{8}$/);
    }
  });
});

describe('normalizeForHash', () => {
  /**
   * Two copies of one quote differ in marker depth, whitespace and often in
   * whether links were rewritten. All three have to fall out of the key, or
   * deduplication never fires.
   */
  it('collapses the differences between two copies of the same quote', () => {
    const a = '> Hallo Welt\n> Siehe https://example.com/a?utm=1';
    const b = '>> hallo   welt\n>> Siehe https://tracking.example/xyz';
    expect(normalizeForHash(a)).toBe(normalizeForHash(b));
  });

  it('still separates genuinely different text', () => {
    expect(normalizeForHash('Hallo Welt')).not.toBe(normalizeForHash('Tschüss Welt'));
  });
});

describe('stripQuoteMarkers', () => {
  it('removes leading markers at any depth', () => {
    expect(stripQuoteMarkers('> > Hallo\n>> Welt')).toBe('Hallo\nWelt');
  });

  it('drops an Outlook divider line', () => {
    const input = '-----Ursprüngliche Nachricht-----\nVon: Lena';
    expect(stripQuoteMarkers(input)).toBe('Von: Lena');
  });

  it('splits a line where a divider was embedded', () => {
    expect(stripQuoteMarkers('Text ---Original Message--- Von: Lena')).toBe('Text\nVon: Lena');
  });
});

describe('quoteIntroFromText and quoteLabel', () => {
  it.each([
    ['Am 1.1.2026 schrieb Lena Becker:', 'german am/schrieb'],
    ['On Jan 1, 2026, Lena Becker wrote:', 'english on/wrote'],
    ['Von: Lena Becker', 'outlook header'],
    ['-- Weitergeleitete Nachricht --', 'forward marker'],
  ])('recognises %s (%s)', (line) => {
    expect(quoteIntroFromText(`> ${line}\n> Hallo`)).toBe(line);
  });

  it('returns nothing when there is no intro line', () => {
    expect(quoteIntroFromText('> just some quoted prose')).toBe('');
  });

  it('falls back to a numbered English label', () => {
    expect(quoteLabel('some quoted prose', 2)).toBe('Quoted message 3');
  });

  it('truncates a very long intro', () => {
    const long = `Am 1.1.2026 schrieb ${'Lena '.repeat(40)}:`;
    const label = quoteLabel(long, 0);
    expect(label).toHaveLength(96);
    expect(label.endsWith('...')).toBe(true);
  });
});

describe('buildQuotePart', () => {
  it('rejects a fragment too short to be a message', () => {
    expect(buildQuotePart({ text: '>' }, 0)).toBeNull();
    expect(buildQuotePart({ text: '> ok' }, 0)).toBeNull();
  });

  it('builds a part whose id follows its content hash', () => {
    const built = part('Hallo, das ist eine zitierte Nachricht.');
    expect(built.id).toBe(`quote-${built.normalizedHash}`);
    expect(built.duplicateCount).toBe(1);
  });

  it('derives text from HTML when the text side is empty', () => {
    const built = buildQuotePart({ html: '<p>Hallo aus dem Markup</p>', text: '' }, 0);
    expect(built?.text).toBe('Hallo aus dem Markup');
  });
});

describe('isSignatureLikeFragment', () => {
  it.each([
    'Mit freundlichen Grüßen\nLena Becker',
    'Mit freundlichen Gruessen\nLena Becker',
    'Viele Grüße\nLena',
    'Beste Grüße\nLena',
    'Herzliche Grüße\nLena',
    'Gruß\nLena',
    'Best regards\nLena',
    'Kind regards\nLena',
    'Cheers\nLena',
    'Sent from my iPhone',
    'Gesendet von meinem Gerät',
  ])('recognises %o as a sign-off', (text) => {
    expect(isSignatureLikeFragment(text)).toBe(true);
  });

  it.each([
    'Sincerely\nLena',
    'Yours sincerely\nLena',
    'Many thanks\nLena',
    'Thank you\nLena',
    'All the best\nLena',
    'Take care\nLena',
    'Cordialement\nLena',
    'Met vriendelijke groet\nLena',
    'Cordiali saluti\nLena',
    'Saludos cordiales\nLena',
    'Med venlig hilsen\nLena',
    'S pozdravem\nPetra',
  ])('recognises %o as a sign-off', (text) => {
    expect(isSignatureLikeFragment(text)).toBe(true);
  });

  it.each(['MfG\nLena', 'LG\nLena', 'VG,\nLena', 'Best,\nLena', 'Thanks!\nLena', 'BR\nLena'])(
    'recognises the short closing %o on a line of its own',
    (text) => {
      expect(isSignatureLikeFragment(text)).toBe(true);
    },
  );

  /**
   * Short closings only count as a whole line. As a prefix they would swallow
   * ordinary sentences, and folding a real fragment into the quote above it
   * hides text the reader was meant to see.
   */
  it.each([
    'LG OLED bestellt, kommt Dienstag an',
    'Best price so far, wir sollten zuschlagen',
    'Thanks to Lena ist der Build wieder grün',
  ])('does not read %o as a sign-off', (text) => {
    expect(isSignatureLikeFragment(text)).toBe(false);
  });

  it('recognises a block dense with contact details', () => {
    expect(isSignatureLikeFragment('Kontakt\nmail@example.com\n+49 123 456789')).toBe(true);
  });

  /**
   * The heuristic must not be tuned to one mailbox: a German business
   * signature is recognised by its shape, not by whose name is in it.
   */
  it.each([
    'Dr. Sabine Vogt\nVogt Consulting GmbH\nPostfach 4711\n+49 221 1234567',
    'Marco Ferrari\nFerrari Studio\nmail@ferrari.example\nhttps://ferrari.example',
  ])('recognises an unfamiliar signature block', (text) => {
    expect(isSignatureLikeFragment(text)).toBe(true);
  });

  /** A quote intro means it is a message, whatever else it contains. */
  it('never treats a fragment with an intro line as a signature', () => {
    expect(isSignatureLikeFragment('Am 1.1.2026 schrieb Lena:\nmail@example.com')).toBe(false);
  });

  it('leaves ordinary prose alone', () => {
    expect(isSignatureLikeFragment('Können wir den Termin auf Dienstag verschieben?')).toBe(false);
  });
});

describe('splitSignatureIntroFragment', () => {
  it('cuts a signature away from the intro line that follows it', () => {
    const split = splitSignatureIntroFragment(
      'Viele Grüße\nAlex\nAm 1.1.2026 schrieb Lena:\nHallo',
    );
    expect(split).not.toBeNull();
    expect(split!.before).toBe('Viele Grüße\nAlex');
    expect(split!.intro).toBe('Am 1.1.2026 schrieb Lena:');
    expect(split!.after.startsWith('Am 1.1.2026')).toBe(true);
  });

  it('does not cut when the part before the intro is not a signature', () => {
    expect(
      splitSignatureIntroFragment('Hier die Details zum Projekt\nAm 1.1.2026 schrieb Lena:\nHallo'),
    ).toBeNull();
  });

  it('does not cut when the intro is the very first line', () => {
    expect(splitSignatureIntroFragment('Am 1.1.2026 schrieb Lena:\nHallo')).toBeNull();
  });
});

describe('dedupeQuoteParts', () => {
  it('counts an identical quote instead of listing it twice', () => {
    const text = 'Hallo, das ist eine zitierte Nachricht.';
    const deduped = dedupeQuoteParts([part(text), part(text)]);
    expect(deduped).toHaveLength(1);
    expect(deduped[0]!.duplicateCount).toBe(2);
  });

  /**
   * The HTML and text alternatives of one message quote the same reply with
   * different whitespace. Equal intro lines catch what the content hash misses.
   */
  it('collapses two parts that share an intro line', () => {
    const a = part('Am 1.1.2026 schrieb Lena:\nHallo Welt, wie geht es dir?');
    const b = part('Am 1.1.2026 schrieb Lena:\nEtwas anderer Text hier drin.');
    const deduped = dedupeQuoteParts([a, b]);
    expect(deduped).toHaveLength(1);
    expect(deduped[0]!.duplicateCount).toBe(2);
  });

  it('keeps distinct quotes that have no intro line', () => {
    const deduped = dedupeQuoteParts([
      part('Erster zitierter Abschnitt hier.'),
      part('Ganz anderer zweiter Abschnitt.'),
    ]);
    expect(deduped).toHaveLength(2);
  });

  it('does not mutate the parts it was given', () => {
    const text = 'Hallo, das ist eine zitierte Nachricht.';
    const input = [part(text), part(text)];
    dedupeQuoteParts(input);
    expect(input[0]!.duplicateCount).toBe(1);
  });
});

describe('mergeQuoteContinuations', () => {
  it('folds a trailing signature into the quote above it', () => {
    const merged = mergeQuoteContinuations([
      part('Am 1.1.2026 schrieb Lena:\nHallo Welt, wie geht es dir?'),
      part('Viele Grüße\nAlex Morgan\nmail@example.com'),
    ]);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.text).toContain('Alex Morgan');
  });

  it('keeps two real messages apart', () => {
    const merged = mergeQuoteContinuations([
      part('Am 1.1.2026 schrieb Lena:\nHallo Welt, wie geht es dir?'),
      part('Am 2.1.2026 schrieb Max:\nMir geht es gut, danke der Nachfrage.'),
    ]);
    expect(merged).toHaveLength(2);
  });

  it('renumbers the fallback labels after merging', () => {
    const merged = mergeQuoteContinuations([
      part('Erster zitierter Abschnitt hier.'),
      part('Zweiter zitierter Abschnitt hier.'),
    ]);
    expect(merged.map((p) => p.label)).toEqual([
      'Quoted message 1',
      'Quoted message 2',
    ]);
  });
});

describe('textFromHtml', () => {
  it('turns breaks and blocks into line breaks', () => {
    expect(textFromHtml('<p>eins</p><p>zwei<br>drei</p>')).toBe('eins\nzwei\ndrei');
  });

  it('normalises non-breaking spaces', () => {
    expect(textFromHtml('<p>a&nbsp;b</p>')).toBe('a b');
  });

  it('returns nothing for empty markup', () => {
    expect(textFromHtml('   ')).toBe('');
  });

  /*
   * The text is read without mutating the tree now. The reference below is the
   * former shape — replace the breaks, append to the blocks, take textContent —
   * because quote hashes are built from this text and must not move.
   */
  function mutatingReference(html: string): string {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    doc.querySelectorAll('br').forEach((node) => node.replaceWith(doc.createTextNode('\n')));
    doc.querySelectorAll('address, article, aside, blockquote, div, footer, header, li, p, section, table, tr')
      .forEach((node) => node.append(doc.createTextNode('\n')));
    return (doc.body?.textContent || '')
      .replace(/\u00a0/g, ' ')
      .replace(/[ \t]+\n/g, '\n')
      .replace(/\n[ \t]+/g, '\n')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  it.each([
    '<div>outer<div>inner<p>para<br>line</p></div>tail</div>',
    '<table><tr><td>a</td><td>b<br></td></tr><tr><td>c</td></tr></table>',
    '<p>text<!-- comment --> more</p><template><p>hidden</p></template>',
    '<style>.x { color: red }</style><ul><li>one</li><li>two&nbsp; </li></ul>',
    '<blockquote type="cite">Am 1. Mai schrieb A:<br><br><br><br>quoted</blockquote>',
    '<svg><text>vector</text></svg><section><header>h</header><footer>f</footer></section>',
  ])('reads the same text as the mutating pass: %s', (html) => {
    expect(textFromHtml(html)).toBe(mutatingReference(html));
  });
});

describe('extractHtmlBody parsing', () => {
  afterEach(() => vi.restoreAllMocks());

  it('parses a message without quotes once', () => {
    const parses = vi.spyOn(DOMParser.prototype, 'parseFromString');
    const result = extractHtmlBody('<div><p>Hallo</p><p>wie besprochen.</p></div>');
    expect(result.quotes).toEqual([]);
    expect(parses).toHaveBeenCalledTimes(1);
  });

  it('still finds plain-text quotes from the tree it parsed', () => {
    const result = extractHtmlBody(
      '<div>Danke!</div><div>Am 1. Mai 2026 schrieb Lena:</div><div>&gt; Kannst du das Angebot bis Freitag schicken?</div>',
    );
    expect(result.quotes).toHaveLength(1);
    expect(result.quotes[0]!.text).toContain('Angebot bis Freitag');
  });
});

describe('extractPlainTextBody', () => {
  it('keeps a message without quotes whole', () => {
    const result = extractPlainTextBody('Hallo Lena,\n\nwie besprochen.');
    expect(result.currentText).toBe('Hallo Lena,\n\nwie besprochen.');
    expect(result.quotes).toEqual([]);
  });

  it('splits the reply from the history below it', () => {
    const result = extractPlainTextBody(
      ['Klingt gut!', '', 'Am 1.1.2026 schrieb Lena:', '> Passt dir Dienstag?'].join('\n'),
    );
    expect(result.currentText).toBe('Klingt gut!');
    expect(result.quotes).toHaveLength(1);
    expect(result.quotes[0]!.label).toBe('Am 1.1.2026 schrieb Lena:');
    expect(result.quotes[0]!.text).toContain('Passt dir Dienstag?');
  });

  /**
   * The boundary search skips line 0 deliberately — a reply that opens with the
   * intro line would otherwise be left with an empty body.
   */
  it('does not treat the very first line as a boundary', () => {
    const result = extractPlainTextBody('Am 1.1.2026 schrieb Lena:\n> Passt dir Dienstag?');
    expect(result.currentText).toBe('Am 1.1.2026 schrieb Lena:');
  });

  it('cuts a longer history into one part per intro line', () => {
    const result = extractPlainTextBody(
      [
        'Ja.',
        '',
        'Am 2.1.2026 schrieb Max:',
        '> Mir geht es gut, danke der Nachfrage.',
        'Am 1.1.2026 schrieb Lena:',
        '> Hallo Welt, wie geht es dir?',
      ].join('\n'),
    );
    expect(result.quotes.map((q) => q.label)).toEqual([
      'Am 2.1.2026 schrieb Max:',
      'Am 1.1.2026 schrieb Lena:',
    ]);
  });

  it('keeps Outlook From and Sent headers in the same quoted message', () => {
    const result = extractPlainTextBody(
      [
        'Aktuelle Antwort.',
        '',
        'Von: Lena Becker',
        'Gesendet: Mittwoch, 5. August 2026 13:30',
        'An: Max Beispiel',
        'Betreff: Re: Termin',
        '',
        'Erste zitierte Nachricht.',
        'Von: Max Beispiel',
        'Gesendet: Freitag, 31. Juli 2026 12:59',
        'An: Lena Becker',
        'Betreff: Termin',
        '',
        'Zweite zitierte Nachricht.',
      ].join('\n'),
    );

    expect(result.quotes).toHaveLength(2);
    expect(result.quotes[0]!.text).toContain('Gesendet: Mittwoch, 5. August 2026 13:30');
    expect(result.quotes[1]!.label).toBe('Von: Max Beispiel');
  });

  it('handles a history marked only with > and no intro line', () => {
    const result = extractPlainTextBody('Kurz:\n> Der zitierte Text steht hier.');
    expect(result.currentText).toBe('Kurz:');
    expect(result.quotes).toHaveLength(1);
    expect(result.quotes[0]!.label).toBe('Quoted message 1');
  });
});

describe('extractHtmlBody', () => {
  it('lifts a Gmail quote out of the body', () => {
    const result = extractHtmlBody(
      '<div>Klingt gut!<div class="gmail_quote">Am 1.1.2026 schrieb Lena:' +
        '<blockquote class="gmail_quote">Passt dir Dienstag?</blockquote></div></div>',
    );
    expect(result.currentHtml).toContain('Klingt gut!');
    expect(result.currentHtml).not.toContain('Passt dir Dienstag?');
    expect(result.quotes.length).toBeGreaterThan(0);
    expect(result.quotes[0]!.text).toContain('Passt dir Dienstag?');
  });

  it('lifts an Apple Mail blockquote out of the body', () => {
    const result = extractHtmlBody(
      '<div>Antwort<blockquote type="cite">Die zitierte Nachricht steht hier.</blockquote></div>',
    );
    expect(result.currentHtml).toContain('Antwort');
    expect(result.currentHtml).not.toContain('Die zitierte Nachricht');
  });

  /**
   * Outlook wraps the intro and the quote in a named section. Regression guard
   * for the loop that kept re-selecting the same descendant: on a detached
   * clone `remove()` is a no-op, so the extractor has to remove a node of the
   * current container to make progress. Before the fix this hung the renderer.
   */
  it('terminates on Outlook reply sections and keeps the intro as the label', () => {
    const result = extractHtmlBody(
      '<div>Kurze Antwort<div name="messageReplySection">Am 1.1.2026 schrieb Lena:' +
        '<blockquote>Passt dir Dienstag doch besser?</blockquote></div></div>',
    );
    expect(result.currentHtml).toContain('Kurze Antwort');
    expect(result.currentHtml).not.toContain('Passt dir Dienstag');
    expect(result.quotes[0]!.label).toBe('Am 1.1.2026 schrieb Lena:');
  });

  it('walks a nested reply chain without losing the inner messages', () => {
    const result = extractHtmlBody(
      '<div>Antwort drei' +
        '<blockquote>Am 2.1.2026 schrieb Max:<div>Zweite Nachricht im Verlauf.' +
        '<blockquote>Am 1.1.2026 schrieb Lena:<div>Erste Nachricht im Verlauf.</div></blockquote>' +
        '</div></blockquote></div>',
    );
    expect(result.currentHtml).toContain('Antwort drei');
    expect(result.currentHtml).not.toContain('Zweite Nachricht');
    expect(result.quotes.length).toBeGreaterThanOrEqual(2);
  });

  it('does not flatten or duplicate an Outlook message before a nested quote', () => {
    const result = extractHtmlBody(
      '<div>Aktuelle Antwort</div>' +
        '<div style="border-left:solid blue 1.5pt">' +
          '<div><p><b>Von:</b> Lena Becker<br><b>Gesendet:</b> Mittwoch, 5. August 2026 13:30<br>' +
            '<b>An:</b> Max Beispiel<br><b>Betreff:</b> Re: Termin</p></div>' +
          '<p>Erste zitierte Nachricht.</p>' +
          '<div style="border-left:solid blue 1.5pt">' +
            '<div><p><b>Von:</b> Max Beispiel<br><b>Gesendet:</b> Freitag, 31. Juli 2026 12:59<br>' +
              '<b>An:</b> Lena Becker<br><b>Betreff:</b> Termin</p></div>' +
            '<p>Zweite zitierte Nachricht.</p>' +
          '</div>' +
        '</div>',
    );

    expect(result.quotes).toHaveLength(2);
    expect(result.quotes.map((quote) => quote.label)).toEqual([
      'Von: Lena Becker',
      'Von: Max Beispiel',
    ]);
    expect(result.quotes.every((quote) => Boolean(quote.html))).toBe(true);
    expect(result.quotes[1]!.html).toContain('<b>Von:</b> Max Beispiel<br>');
    expect(result.quotes[1]!.html).not.toContain('Lena Becker Gesendet:');
  });

  it('does not render an unmarked inner Outlook message a second time as text', () => {
    const result = extractHtmlBody(
      '<div>Aktuelle Antwort</div>' +
        '<div style="border-left:solid blue 1.5pt">' +
          '<p><b>Von:</b> Lena Becker<br><b>Gesendet:</b> Mittwoch, 5. August 2026 13:30</p>' +
          '<p>Erste zitierte Nachricht.</p>' +
          '<div><p><b>Von:</b> Max Beispiel<br><b>Gesendet:</b> Freitag, 31. Juli 2026 12:59</p>' +
            '<p>Zweite zitierte Nachricht.</p></div>' +
        '</div>',
    );

    expect(result.quotes).toHaveLength(1);
    expect(result.quotes[0]!.html).toContain('Zweite zitierte Nachricht.');
  });

  it('keeps distinct Outlook messages from the same sender', () => {
    const result = extractHtmlBody(
      '<div>Aktuelle Antwort</div>' +
        '<div style="border-left:solid blue 1.5pt">' +
          '<p><b>Von:</b> Lena Becker<br><b>Gesendet:</b> Mittwoch, 5. August 2026 13:30</p>' +
          '<p>Neuere zitierte Nachricht.</p>' +
          '<div style="border-left:solid blue 1.5pt">' +
            '<p><b>Von:</b> Lena Becker<br><b>Gesendet:</b> Freitag, 31. Juli 2026 09:34</p>' +
            '<p>Ältere zitierte Nachricht.</p>' +
          '</div>' +
        '</div>',
    );

    expect(result.quotes).toHaveLength(2);
    expect(result.quotes.map((quote) => quote.text)).toEqual([
      expect.stringContaining('Neuere zitierte Nachricht.'),
      expect.stringContaining('Ältere zitierte Nachricht.'),
    ]);
  });

  /** Malformed or adversarial nesting must not exhaust the renderer. */
  it('survives deeply nested markup', () => {
    const depth = 200;
    const html = `<div>Antwort${'<blockquote>x'.repeat(depth)}${'</blockquote>'.repeat(depth)}</div>`;
    const result = extractHtmlBody(html);
    expect(result.currentHtml).toContain('Antwort');
    expect(result.quotes.length).toBeGreaterThan(0);
  });

  it('preserves head styles ahead of the body markup', () => {
    const result = extractHtmlBody(
      '<html><head><style>.a{color:red}</style></head><body><p>Hallo</p></body></html>',
    );
    expect(result.currentHtml?.startsWith('<style>')).toBe(true);
    expect(result.currentHtml).toContain('<p>Hallo</p>');
  });

  it('reports no quotes for a message that has none', () => {
    const result = extractHtmlBody('<div><p>Nur eine einfache Nachricht.</p></div>');
    expect(result.quotes).toEqual([]);
  });

  /**
   * Both extraction paths run over an HTML message: some clients mark history
   * only with `>` inside the HTML, where no selector matches. The same reply
   * must not then be listed twice.
   */
  it('does not list a quote twice when both paths find it', () => {
    const result = extractHtmlBody(
      '<div>Klingt gut!<blockquote>Am 1.1.2026 schrieb Lena:<br>&gt; Passt dir Dienstag?</blockquote></div>',
    );
    const labels = result.quotes.map((q) => q.label);
    expect(new Set(labels).size).toBe(labels.length);
  });
});
