import { describe, expect, it } from 'vitest';
import {
  authenticationVerdict,
  hasAuthenticationResults,
  summarizeAuthentication,
} from './messageAuthentication';

describe('message authentication', () => {
  it('maps provider verdicts to a tone and label key', () => {
    expect(authenticationVerdict('SPF', ' PASS ')).toMatchObject({ key: 'pass', tone: 'pass' });
    expect(authenticationVerdict('DMARC', 'policy')).toMatchObject({ key: 'policy', tone: 'fail' });
    expect(authenticationVerdict('SPF', 'softfail')).toMatchObject({ key: 'softfail', tone: 'warn' });
    expect(authenticationVerdict('DKIM', null)).toMatchObject({ key: 'unavailable', tone: 'none' });
    expect(authenticationVerdict('DKIM', 'bestguesspass')).toMatchObject({
      key: 'providerResult',
      tone: 'none',
      raw: 'bestguesspass',
    });
  });

  it('lets the worst verdict decide the summary', () => {
    expect(summarizeAuthentication({ spf: 'pass', dkim: 'pass', dmarc: 'fail' })).toBe('fail');
    expect(summarizeAuthentication({ spf: 'softfail', dkim: 'pass' })).toBe('warn');
    expect(summarizeAuthentication({ dkim: 'pass', dmarc: 'none' })).toBe('pass');
    expect(summarizeAuthentication({ spf: 'neutral', dmarc: 'temperror' })).toBe('none');
    expect(summarizeAuthentication({})).toBe('none');
  });

  it('treats blank verdicts as absent', () => {
    expect(hasAuthenticationResults({ spf: ' ', dkim: null })).toBe(false);
    expect(hasAuthenticationResults({ dmarc: 'none' })).toBe(true);
  });
});
