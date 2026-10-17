import { t } from '@/i18n/runtime';
import { transportErrorClass, type TransportErrorClass } from '@/utils/transportLabels';

/**
 * The reauth banner's wording for the two OAuth failures it can explain.
 *
 * The banner shows what the provider last told us about a stored sign-in, so
 * anything outside those two stays visible verbatim rather than being softened
 * into a transport sentence that may not be true here.
 */
const OAUTH_KEYS: Partial<Record<TransportErrorClass, string>> = {
  oauthExpired: 'errors.oauth.invalidGrant',
  oauthConfigMissing: 'errors.oauth.missingConfig',
};

export function authErrorLabel(error?: string | null): string {
  if (!error) return '';
  const errorClass = transportErrorClass(error);
  const key = errorClass && OAUTH_KEYS[errorClass];
  return key ? t(key) : error;
}
