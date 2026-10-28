// An identity palette, not theme colours: the colour is picked by hashing the
// sender, so it has to stay the same value in both themes to stay recognisable.
const avatarColors = [
  '#5196ff',
  '#4cd68a',
  '#f0b232',
  '#a97fff',
  '#3ad4e5',
  '#f06898',
  '#ff9f43',
  '#ff5a5a',
];

const faviconDomains: Record<string, string> = {
  'google.com': 'google.com',
  'aistudio.google.com': 'google.com',
  'cloud.google.com': 'google.com',
  'googlemail.com': 'google.com',
  'email.apple.com': 'apple.com',
  'github.com': 'github.com',
  'sentry.io': 'sentry.io',
  'linear.app': 'linear.app',
  'stripe.com': 'stripe.com',
  'dhl.de': 'dhl.de',
  'amazon.de': 'amazon.de',
  'metro.de': 'metro.de',
  'linkedin.com': 'linkedin.com',
  'kickstarter.com': 'kickstarter.com',
  'producthunt.com': 'producthunt.com',
  'pragmaticengineer.com': 'pragmaticengineer.com',
  'hackernewsletter.com': 'hackernewsletter.com',
  'expedia.com': 'expedia.com',
  'vercel.com': 'vercel.com',
  'x.com': 'x.com',
};

// Vendored logos stay sharp and work offline. Contact photos take precedence.
const localBrandAvatars: Record<string, string> = {
  'google.com': '/avatars/google.svg',
  'github.com': '/avatars/github.svg',
  'apple.com': '/avatars/apple.svg',
  'sentry.io': '/avatars/sentry.svg',
  'linear.app': '/avatars/linear.svg',
  'stripe.com': '/avatars/stripe.svg',
  'dhl.de': '/avatars/dhl.svg',
  'kickstarter.com': '/avatars/kickstarter.svg',
  'producthunt.com': '/avatars/producthunt.svg',
  'expedia.com': '/avatars/expedia.svg',
  'vercel.com': '/avatars/vercel.svg',
  'x.com': '/avatars/x.svg',
  'amazon.de': '/avatars/amazon.svg',
  'metro.de': '/avatars/metro.svg',
  'linkedin.com': '/avatars/linkedin.svg',
  'pragmaticengineer.com': '/avatars/pragmaticengineer.svg',
};

/*
 * The monograms (see public/avatars/sources.json) are whole tiles with their
 * own fill. Set on the white logo plate like the glyph logos above, they left a
 * thick white ring around a grey disc — the brightest thing in a dark list.
 * They fill the avatar instead.
 */
const monogramTiles: ReadonlySet<string> = new Set([
  'google.com',
  'github.com',
  'apple.com',
  'amazon.de',
  'metro.de',
  'linkedin.com',
  'pragmaticengineer.com',
]);

export interface SenderAvatar {
  initials: string;
  color: string;
  imageUrl: string | null;
  domain: string | null;
  source: 'provided' | 'favicon' | 'initials';
  /** The image is a full tile that fills the avatar, not a logo set on a plate. */
  tile: boolean;
}

export function resolveSenderAvatar(input: {
  name: string;
  email?: string | null;
  imageUrl?: string | null;
  imageSource?: string | null;
  size?: number;
}): SenderAvatar {
  const label = input.name.trim() || input.email?.trim() || '?';
  const domain = domainFromEmail(input.email);
  const faviconDomain = domain ? resolveFaviconDomain(domain) : null;
  const brandImage = faviconDomain ? localBrandAvatars[faviconDomain] : null;
  const providedPhoto = input.imageUrl && input.imageSource !== 'favicon';

  const imageUrl = providedPhoto ? input.imageUrl! : brandImage || input.imageUrl || null;

  return {
    initials: initialsFor(label),
    color: colorFor(label),
    imageUrl,
    domain,
    source: input.imageUrl
      ? input.imageSource === 'favicon' ? 'favicon' : 'provided'
      : brandImage ? 'favicon' : 'initials',
    tile: !providedPhoto && Boolean(brandImage) && monogramTiles.has(faviconDomain!),
  };
}

function domainFromEmail(email?: string | null): string | null {
  const raw = email?.trim().toLowerCase();
  const domain = raw?.split('@')[1]?.replace(/^www\./, '') ?? '';
  if (!domain || domain.endsWith('.local') || domain.endsWith('.example') || domain.endsWith('.dev')) {
    return null;
  }
  return domain;
}

function resolveFaviconDomain(domain: string): string | null {
  const match = Object.entries(faviconDomains).find(([suffix]) =>
    domain === suffix || domain.endsWith(`.${suffix}`),
  );
  if (match) {
    return match[1];
  }
  return null;
}

export function initialsFor(name: string): string {
  return name
    .split(/[\s._-]+/)
    .filter(Boolean)
    .map((part) => part[0])
    .join('')
    .slice(0, 2)
    .toUpperCase();
}

function colorFor(value: string): string {
  let hash = 0;
  for (let i = 0; i < value.length; i++) {
    hash = ((hash << 5) - hash + value.charCodeAt(i)) | 0;
  }
  // Modulo keeps the index inside the palette.
  return avatarColors[Math.abs(hash) % avatarColors.length] ?? avatarColors[0]!;
}
