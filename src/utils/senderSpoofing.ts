import { t } from '@/i18n/runtime';
import { senderDomain } from './senderDomains';
import { toUnicode } from 'punycode/';
import confusables from './data/confusablesAscii.json';

interface BrandEntry {
  label: string;
  tokens: string[];
  domains: string[];
}

const BRANDS: BrandEntry[] = [
  { label: 'N26', tokens: ['n26'], domains: ['n26.com', 'n26.de'] },
  { label: 'PayPal', tokens: ['paypal'], domains: ['paypal.com', 'paypal.de'] },
  { label: 'Amazon', tokens: ['amazon'], domains: ['amazon.de', 'amazon.com'] },
  { label: 'Apple', tokens: ['apple'], domains: ['apple.com', 'icloud.com', 'me.com'] },
  {
    label: 'Google',
    tokens: ['google'],
    domains: ['google.com', 'googlemail.com', 'gmail.com', 'android.com'],
  },
  {
    label: 'Microsoft',
    tokens: ['microsoft'],
    domains: ['microsoft.com', 'outlook.com', 'live.com', 'hotmail.com', 'office.com'],
  },
  { label: 'Netflix', tokens: ['netflix'], domains: ['netflix.com'] },
  { label: 'Facebook', tokens: ['facebook'], domains: ['facebook.com', 'facebookmail.com'] },
  { label: 'Instagram', tokens: ['instagram'], domains: ['instagram.com'] },
  { label: 'WhatsApp', tokens: ['whatsapp'], domains: ['whatsapp.com'] },
  { label: 'LinkedIn', tokens: ['linkedin'], domains: ['linkedin.com'] },
  { label: 'X', tokens: ['twitter'], domains: ['x.com', 'twitter.com'] },
  { label: 'DHL', tokens: ['dhl'], domains: ['dhl.de', 'dhl.com'] },
  { label: 'Hermes', tokens: ['hermes'], domains: ['hermes.de', 'hermesworld.com'] },
  { label: 'DPD', tokens: ['dpd'], domains: ['dpd.de', 'dpd.com'] },
  { label: 'GLS', tokens: ['gls'], domains: ['gls.de', 'gls-group.eu', 'gls-one.de'] },
  { label: 'UPS', tokens: ['ups'], domains: ['ups.com'] },
  { label: 'FedEx', tokens: ['fedex'], domains: ['fedex.com'] },
  { label: 'Sparkasse', tokens: ['sparkasse'], domains: ['sparkasse.de'] },
  { label: 'ING', tokens: ['ing'], domains: ['ing.de', 'ing-diba.de'] },
  { label: 'Commerzbank', tokens: ['commerzbank'], domains: ['commerzbank.de', 'commerzbank.com'] },
  { label: 'Comdirect', tokens: ['comdirect'], domains: ['comdirect.de'] },
  { label: 'Postbank', tokens: ['postbank'], domains: ['postbank.de'] },
  { label: 'DKB', tokens: ['dkb'], domains: ['dkb.de'] },
  { label: 'Santander', tokens: ['santander'], domains: ['santander.de', 'santander.com'] },
  { label: 'Revolut', tokens: ['revolut'], domains: ['revolut.com'] },
  { label: 'Klarna', tokens: ['klarna'], domains: ['klarna.com', 'klarna.de'] },
  { label: 'Vodafone', tokens: ['vodafone'], domains: ['vodafone.de', 'vodafone.com'] },
  { label: 'Telekom', tokens: ['telekom'], domains: ['telekom.de', 't-online.de'] },
  { label: 'O2', tokens: ['o2'], domains: ['o2online.de', 'o2.de'] },
  { label: 'eBay', tokens: ['ebay'], domains: ['ebay.de', 'ebay.com'] },
  { label: 'Kleinanzeigen', tokens: ['kleinanzeigen'], domains: ['kleinanzeigen.de'] },
  { label: 'Zalando', tokens: ['zalando'], domains: ['zalando.de', 'zalando.com'] },
  { label: 'Lufthansa', tokens: ['lufthansa'], domains: ['lufthansa.com', 'lufthansa.de'] },
  { label: 'Booking.com', tokens: ['booking'], domains: ['booking.com'] },
  { label: 'Airbnb', tokens: ['airbnb'], domains: ['airbnb.com', 'airbnb.de'] },
  { label: 'Steam', tokens: ['steam'], domains: ['steampowered.com', 'steamcommunity.com'] },
  { label: 'Deutsche Bahn', tokens: ['bahn'], domains: ['bahn.de'] },
  { label: 'Ikea', tokens: ['ikea'], domains: ['ikea.com'] },
];

export type SenderAuthResults = {
  spf?: string | null;
  dkim?: string | null;
  dmarc?: string | null;
};

export interface SenderSpoofingWarning {
  kind: 'brand' | 'auth';
  /** Brand label the display name claims; only set for kind 'brand'. */
  brand?: string;
  /** Actual sender domain that does not belong to the brand; only set for kind 'brand'. */
  domain?: string;
  /** Failed mechanism names, only set for kind 'auth'. */
  mechanisms?: string[];
  text: string;
}

// ASCII-target mappings from Unicode UTS #39, version 18.0.0.
// Source: https://www.unicode.org/Public/security/latest/confusables.txt (2026-08-06 snapshot)
// Keys eliminated by NFKC, format stripping or ASCII case folding are omitted.
function spoofingSkeleton(value: string): string {
  // Unicode glyphs retain their case until mapped; Greek capital Mu resembles M.
  const normalized = value.normalize('NFKC').replace(/\p{Cf}/gu, '').replace(/[A-Z]/g, (letter) => letter.toLowerCase());
  const mappings: Record<string, string> = confusables;
  const skeleton = Array.from(normalized, (character) => mappings[character] ?? character).join('').toLowerCase();
  // Fold ASCII emitted by mappings too, such as Mu becoming m, then rn.
  return Array.from(skeleton, (character) => (mappings[character] ?? character).toLowerCase()).join('');
}

/*
 * The brand side of the comparison never changes. Skeletonizing it per call
 * cost the new-sender review stack most of its time: one call per sender, on
 * every list mutation.
 */
let brandSkeletons: Array<{ tokens: string[]; domains: string[] }> | null = null;
function skeletonsForBrands(): Array<{ tokens: string[]; domains: string[] }> {
  brandSkeletons ??= BRANDS.map((brand) => ({
    tokens: brand.tokens.map(spoofingSkeleton),
    domains: brand.domains.map(spoofingSkeleton),
  }));
  return brandSkeletons;
}

function nameTokens(displayName: string): string[] {
  return spoofingSkeleton(displayName).split(/[^\p{L}\p{N}]+/u).filter(Boolean);
}

function domainMatches(senderDomain: string, officialDomains: string[]): boolean {
  return officialDomains.some(
    (official) => senderDomain === official || senderDomain.endsWith(`.${official}`),
  );
}

function authFailures(auth: SenderAuthResults): string[] {
  const failures: string[] = [];
  if (auth.dmarc?.toLowerCase() === 'fail') {
    failures.push('DMARC');
  }
  const spfFailed = auth.spf?.toLowerCase() === 'fail';
  const dkimPassed = auth.dkim?.toLowerCase() === 'pass';
  if (spfFailed && !dkimPassed) {
    failures.push('SPF');
  }
  return failures;
}

export function detectSenderSpoofing(
  displayName: string | null | undefined,
  fromEmail: string,
  auth?: SenderAuthResults,
): SenderSpoofingWarning | null {
  const domain = senderDomain(fromEmail);

  if (domain) {
    const tokens = nameTokens(displayName ?? '');
    let unicodeDomain = fromEmail.slice(fromEmail.lastIndexOf('@') + 1).trim();
    try {
      unicodeDomain = toUnicode(unicodeDomain);
    } catch {
      // Malformed sender domains must not prevent reading the message.
    }
    const domainSkeleton = spoofingSkeleton(unicodeDomain);
    const skeletons = skeletonsForBrands();
    for (const [index, brand] of BRANDS.entries()) {
      const skeleton = skeletons[index]!;
      const nameClaimsBrand = skeleton.tokens.some((token) => tokens.includes(token));
      const domainImitatesBrand = domainMatches(domainSkeleton, skeleton.domains);
      if ((nameClaimsBrand || domainImitatesBrand) && !domainMatches(domain, brand.domains)) {
        return {
          kind: 'brand', brand: brand.label, domain,
          text: nameClaimsBrand
            ? t('mail.spoofing.brand', { brand: brand.label, domain })
            : t('mail.spoofing.domain', { brand: brand.label, domain }),
        };
      }
    }
  }

  const failures = auth ? authFailures(auth) : [];
  if (failures.length > 0) {
    return {
      kind: 'auth',
      mechanisms: failures,
      text: t('mail.spoofing.auth', {
        mechanisms: failures.join(t('mail.spoofing.mechanismJoin')),
      }),
    };
  }

  return null;
}
