// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { mailGist, trimMailBodyHtml, trimMailBodyText } from './mailGist';

describe('mailGist', () => {
  it('drops greeting, closing and signature, keeps the content', () => {
    const text = [
      'Hallo Alex,',
      '',
      'der Statusbericht für KW 30 ist fertig, Quoten wie besprochen.',
      '',
      'Mit freundlichen Grüßen',
      'Mara Vogel',
      'Stadt Beispielstadt',
    ].join('\n');
    expect(mailGist(text)).toBe('der Statusbericht für KW 30 ist fertig, Quoten wie besprochen.');
  });

  it('cuts at the signature delimiter', () => {
    const text = ['Kurzer Zwischenstand: Rollback läuft.', '-- ', 'Sarah Kim', 'App.io Ops'].join(
      '\n',
    );
    expect(mailGist(text)).toBe('Kurzer Zwischenstand: Rollback läuft.');
  });

  it('drops quoted history and reply headers', () => {
    const text = [
      'Hi,',
      'passt für mich, Termin steht.',
      '',
      'Am 22.07.2026 um 14:18 schrieb Mara Vogel:',
      '> Hallo Alex,',
      '> können wir den Termin bestätigen?',
    ].join('\n');
    expect(mailGist(text)).toBe('passt für mich, Termin steht.');
  });

  it('does not treat a closing phrase mid-sentence as the end', () => {
    const text = ['Viele Grüße an das Team bitte ausrichten.', 'Der Rest folgt morgen.'].join('\n');
    expect(mailGist(text)).toContain('Der Rest folgt morgen.');
  });

  it('falls back to the original text when everything would be stripped', () => {
    expect(mailGist('Hallo Alex,')).toBe('Hallo Alex,');
    expect(mailGist('')).toBe('');
  });
});

describe('formatted mail body trimming', () => {
  it('keeps text layout while dropping greeting, signature and history', () => {
    const text = [
      'Hallo Alex,',
      '',
      'Erste Zeile.',
      'Zweite Zeile.',
      '',
      'Viele Grüße',
      'Lena',
      '',
      'Am 1.1.2026 schrieb Alex:',
      '> Alter Text',
    ].join('\n');

    expect(trimMailBodyText(text)).toBe('Erste Zeile.\nZweite Zeile.');
  });

  it('keeps HTML formatting while dropping greeting and signature', () => {
    const html = [
      '<style>.important{font-weight:bold}</style>',
      '<div><p>Hallo Alex,</p>',
      '<p class="important">Erste Zeile.<br>Zweite Zeile.</p>',
      '<p>Mit freundlichen Grüßen</p><p>Lena</p></div>',
    ].join('');

    const result = trimMailBodyHtml(html);
    expect(result).toContain('<style>.important{font-weight:bold}</style>');
    expect(result).toContain('<p class="important">Erste Zeile.<br>Zweite Zeile.</p>');
    expect(result).not.toContain('Hallo Alex');
    expect(result).not.toContain('Lena');
  });

  it('removes a client-marked signature without flattening the body', () => {
    const result = trimMailBodyHtml(
      '<div><strong>Status:</strong> erledigt.</div><div class="gmail_signature">Lena GmbH</div>',
    );

    expect(result).toContain('<strong>Status:</strong> erledigt.');
    expect(result).not.toContain('Lena GmbH');
  });
});
