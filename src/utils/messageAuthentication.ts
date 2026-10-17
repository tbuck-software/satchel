export type AuthenticationMechanism = 'SPF' | 'DKIM' | 'DMARC';

/** How a single verdict should read: green, red, amber, or muted. */
export type AuthenticationTone = 'pass' | 'fail' | 'warn' | 'none';

export type AuthenticationVerdictKey =
  | 'pass'
  | 'fail'
  | 'policy'
  | 'permerror'
  | 'softfail'
  | 'neutral'
  | 'temperror'
  | 'none'
  | 'unavailable'
  | 'providerResult';

export interface AuthenticationVerdict {
  mechanism: AuthenticationMechanism;
  /** i18n key under `mail.authentication.*`. */
  key: AuthenticationVerdictKey;
  tone: AuthenticationTone;
  /** Raw provider result, only set for results we do not recognise. */
  raw: string;
}

export interface AuthenticationResults {
  spf?: string | null;
  dkim?: string | null;
  dmarc?: string | null;
}

const VERDICTS: Record<string, { key: AuthenticationVerdictKey; tone: AuthenticationTone }> = {
  pass: { key: 'pass', tone: 'pass' },
  fail: { key: 'fail', tone: 'fail' },
  policy: { key: 'policy', tone: 'fail' },
  permerror: { key: 'permerror', tone: 'fail' },
  softfail: { key: 'softfail', tone: 'warn' },
  neutral: { key: 'neutral', tone: 'none' },
  temperror: { key: 'temperror', tone: 'none' },
  none: { key: 'none', tone: 'none' },
  '': { key: 'unavailable', tone: 'none' },
};

export function authenticationVerdict(
  mechanism: AuthenticationMechanism,
  raw: string | null | undefined,
): AuthenticationVerdict {
  const normalized = raw?.trim().toLowerCase() || '';
  const known = VERDICTS[normalized];
  if (known) return { mechanism, ...known, raw: normalized };
  return { mechanism, key: 'providerResult', tone: 'none', raw: normalized };
}

export function authenticationVerdicts(results: AuthenticationResults): AuthenticationVerdict[] {
  return [
    authenticationVerdict('SPF', results.spf),
    authenticationVerdict('DKIM', results.dkim),
    authenticationVerdict('DMARC', results.dmarc),
  ];
}

export function hasAuthenticationResults(results: AuthenticationResults): boolean {
  return Boolean(results.spf?.trim() || results.dkim?.trim() || results.dmarc?.trim());
}

/** Worst verdict wins; a lone pass still counts as pass. */
export function summarizeAuthentication(results: AuthenticationResults): AuthenticationTone {
  const tones = authenticationVerdicts(results).map((verdict) => verdict.tone);
  if (tones.includes('fail')) return 'fail';
  if (tones.includes('warn')) return 'warn';
  if (tones.includes('pass')) return 'pass';
  return 'none';
}
