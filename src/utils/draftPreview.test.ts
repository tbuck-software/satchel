// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { draftPreviewText } from './draftPreview';

describe('draftPreviewText', () => {
  it('keeps what was written and drops the quoted reply history', () => {
    const bodyText = [
      'The OAuth fix is on staging.',
      '',
      'Can you check the login again?',
      '',
      'On Tue, Oct 27, 2026 at 07:11, Sarah Kim wrote:',
      '> Users cannot sign in on staging.',
      '> Steps: open staging login.',
    ].join('\n');

    expect(draftPreviewText({ bodyText })).toBe('The OAuth fix is on staging.\n\nCan you check the login again?');
  });

  it('stops at a quote no composer header introduced', () => {
    expect(draftPreviewText({ bodyText: 'Agreed.\r\n> earlier text' })).toBe('Agreed.');
  });

  it('reads an HTML-only draft', () => {
    expect(draftPreviewText({ bodyText: '  ', bodyHtml: '<p>First line</p><p>Second line</p>' }))
      .toBe('First line\nSecond line');
  });

  it('is empty for a draft with nothing written yet', () => {
    expect(draftPreviewText({ bodyText: '\nAm 27.10.2026 um 07:11 schrieb Sarah Kim:\n> Hallo' })).toBe('');
  });
});
