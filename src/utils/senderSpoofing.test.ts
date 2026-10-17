import { describe, expect, it } from 'vitest';
import { detectSenderSpoofing } from './senderSpoofing';

describe('detectSenderSpoofing', () => {
  it.each(['Pаypal', 'Pay\u200bPal', 'ＰａｙＰａｌ', 'Pаy\u2060Pal'])('detects Unicode brand impersonation: %s', (name) => {
    expect(detectSenderSpoofing(name, 'billing@attacker.example')?.brand).toBe('PayPal');
  });
  it.each(['service@paypаl.com', 'service@xn--pple-43d.com', 'service@paypa1.com'])('detects a lookalike domain without a display name: %s', (email) => {
    expect(detectSenderSpoofing(null, email)?.kind).toBe('brand');
  });
  it('does not flag unrelated internationalised domains', () => {
    expect(detectSenderSpoofing('Müller', 'info@müller.de')).toBeNull();
  });
  it('handles malformed punycode without breaking the message view', () => {
    expect(() => detectSenderSpoofing(null, 'sender@xn--$.com')).not.toThrow();
  });
  it.each(['Μicrosoft', 'Мicrosoft'])('preserves Unicode glyph case while detecting %s', (name) => {
    expect(detectSenderSpoofing(name, 'billing@attacker.example')?.brand).toBe('Microsoft');
  });
  it.each(['billing@Μicrosoft.com', 'billing@Мicrosoft.com'])('checks displayed domain glyphs in %s', (email) => {
    expect(detectSenderSpoofing(null, email)?.brand).toBe('Microsoft');
  });
  it('flags a brand display name from an unrelated domain', () => {
    const warning = detectSenderSpoofing('N26', 'kundensicherheitscenter@hashtagmediapoint.com');

    expect(warning?.kind).toBe('brand');
    expect(warning?.brand).toBe('N26');
    expect(warning?.domain).toBe('hashtagmediapoint.com');
  });

  it('accepts official and subdomain senders of the claimed brand', () => {
    expect(detectSenderSpoofing('N26 Kundenservice', 'service@n26.com')).toBeNull();
    expect(detectSenderSpoofing('PayPal', 'noreply@mail.paypal.de')).toBeNull();
  });

  it('matches brand tokens as whole words only', () => {
    expect(detectSenderSpoofing('N261 Fanclub', 'someone@random.example')).toBeNull();
    expect(detectSenderSpoofing('DHL Express', 'shipping@lookalike.example')).not.toBeNull();
  });

  it('flags failed DMARC even without a brand name', () => {
    const warning = detectSenderSpoofing('Max Mustermann', 'max@stranger.example', {
      spf: 'pass',
      dkim: 'fail',
      dmarc: 'fail',
    });

    expect(warning?.kind).toBe('auth');
    expect(warning?.mechanisms).toEqual(['DMARC']);
  });

  it('flags SPF failure only when DKIM does not rescue the message', () => {
    expect(
      detectSenderSpoofing('Max Mustermann', 'max@stranger.example', { spf: 'fail', dkim: 'pass' }),
    ).toBeNull();
    const warning = detectSenderSpoofing('Max Mustermann', 'max@stranger.example', {
      spf: 'fail',
      dkim: 'none',
    });
    expect(warning?.mechanisms).toEqual(['SPF']);
  });

  it('stays quiet for softfail, neutral and missing verdicts', () => {
    expect(
      detectSenderSpoofing('Max Mustermann', 'max@stranger.example', {
        spf: 'softfail',
        dkim: 'neutral',
        dmarc: 'none',
      }),
    ).toBeNull();
    expect(detectSenderSpoofing('Max Mustermann', 'max@stranger.example')).toBeNull();
  });

  it('prefers the brand warning over the auth warning', () => {
    const warning = detectSenderSpoofing('N26 Sicherheit', 'alert@hashtagmediapoint.com', {
      dmarc: 'fail',
    });

    expect(warning?.kind).toBe('brand');
  });
});
