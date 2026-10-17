import { describe, expect, it } from 'vitest';
import { icsDescriptionToText, isKnownInviteBoilerplate } from './inviteText';

describe('icsDescriptionToText', () => {
  it('passes plain text through unchanged', () => {
    expect(icsDescriptionToText('Agenda\n- tbd (gerne Vorschläge einreichen)')).toBe(
      'Agenda\n- tbd (gerne Vorschläge einreichen)',
    );
  });

  it('flattens HTML fragments to newlines and strips tags', () => {
    const zoom =
      '<p><br>──────────<br><br>Stefan lädt Sie zu einem Zoom-Meeting ein.<br><br>' +
      'Beitreten:<br><a href="https://example.zoom-x.de/j/1">https://example.zoom-x.de/j/1</a></p>';
    expect(icsDescriptionToText(zoom)).toBe(
      'Stefan lädt Sie zu einem Zoom-Meeting ein.\n\nBeitreten:\nhttps://example.zoom-x.de/j/1',
    );
  });

  it('decodes entities only in HTML input', () => {
    expect(icsDescriptionToText('<b>Kenncode</b>: 4711 &amp; 0815')).toBe('Kenncode: 4711 & 0815');
    expect(icsDescriptionToText('4711 &amp; 0815')).toBe('4711 &amp; 0815');
  });

  it('drops decorative divider lines and collapses blank runs', () => {
    expect(icsDescriptionToText('Hallo\n──────────\n\n\n\nWelt')).toBe('Hallo\n\nWelt');
  });
});

describe('isKnownInviteBoilerplate', () => {
  it('recognizes Exchange-style field sheets', () => {
    const sheet = [
      'Item Type:  Appointment\r',
      'Start Date:  Wednesday, 5 Aug 2026, 02:30:00pm (GMT+0200)\r',
      'Duration:  1 Hour\r',
      'Place:  https://meeting.example/j/1234567890?pwd=fixture-only\r',
      '        fixture-continuation&omn=12345678901\r',
      'Action:  Cancelled\r',
    ].join('\n');
    expect(isKnownInviteBoilerplate(sheet)).toBe(true);
  });

  it('recognizes Google Calendar mails by their footer link', () => {
    const google =
      'Termin\nMittwoch 5. Aug. 2026 ⋅ 14:30 – 15:30\n\nWeitere Informationen\n' +
      'https://support.google.com/calendar/answer/37135#forwarding';
    expect(isKnownInviteBoilerplate(google)).toBe(true);
  });

  it('recognizes group-calendar notifications', () => {
    const notice =
      'Liebe Gruppenteilnehmerin, lieber Gruppenteilnehmer,\n\n' +
      'ein neuer Termin ist in der Gruppe „Beispielteam“ angelegt worden.\n\nTermindetails\nTitel: Sitzung';
    expect(isKnownInviteBoilerplate(notice)).toBe(true);
  });

  it('recognizes a bare cancellation sentence', () => {
    expect(isKnownInviteBoilerplate('Dieser Termin wurde abgesagt.')).toBe(true);
    expect(isKnownInviteBoilerplate('This event has been canceled.')).toBe(true);
  });

  it('keeps hand-written mails visible', () => {
    const mail =
      'Liebes Beispiel-Team,\n\nda unsere DFN-Terminfindung deutlich macht, dass es schwierig ist,\n' +
      'einen Zeitslot zu finden, ist die Wahl auf Mi, den 22. April gefallen.\n\nViele Grüße\nAlex';
    expect(isKnownInviteBoilerplate(mail)).toBe(false);
  });

  it('keeps field sheets with free-text additions visible', () => {
    const mixed =
      'Hi zusammen, kurze Vorbesprechung vorab!\n\nItem Type:  Appointment\nStart Date:  Monday\nDuration:  1 Hour';
    expect(isKnownInviteBoilerplate(mixed)).toBe(false);
  });

  it('treats empty bodies as not classifiable', () => {
    expect(isKnownInviteBoilerplate('   ')).toBe(false);
  });
});
