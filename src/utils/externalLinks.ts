import { shallowRef } from 'vue';

/*
 * `tldts` carries the public-suffix table — around a third of the viewer
 * window's initial JavaScript, and ~46 KiB gzip the main window used to fetch
 * and parse during startup, because this module sits behind the settings
 * store. It is loaded on first use, or warmed once the app is idle, and until
 * it arrives a display/target mismatch counts as a mismatch: the confirmation
 * prompt, never silent approval.
 */
type SuffixOptions = { allowPrivateDomains: boolean; extractHostname: boolean };
type GetDomain = (host: string, options: SuffixOptions) => string | null;
type ParseHost = (host: string, options: SuffixOptions) => { isIcann: boolean | null; isPrivate: boolean | null };
type SuffixTable = { getDomain: GetDomain; parseHost: ParseHost };

let table: SuffixTable | null = null;
let publicSuffixTable: Promise<void> | null = null;
/** Reactive, so a view that answered without the table renders again once it is there. */
const tableLoaded = shallowRef(false);
let provisionalAnswers = 0;

/** Starts loading the public-suffix table; resolves once it is usable. */
export function loadPublicSuffixTable(): Promise<void> {
  publicSuffixTable ??= import('tldts').then((module) => {
    table = { getDomain: module.getDomain as GetDomain, parseHost: module.parse as ParseHost };
    tableLoaded.value = true;
  });
  return publicSuffixTable;
}

/** Whether the table is usable. A reactive read: computeds and renders track it. */
export function publicSuffixTableLoaded(): boolean {
  return tableLoaded.value;
}

/**
 * Counts answers given without the table. A caller that caches what it built
 * from assessments compares the count before and after to learn whether the
 * result has to be redone once `publicSuffixTableLoaded()` turns true.
 */
export function provisionalLinkAnswers(): number {
  return provisionalAnswers;
}

/**
 * The table, or null while it is loading. Link assessment asks for it to be
 * loaded; `linkRegistrableDomain` does not, because the settings store
 * normalizes stored hosts with it at startup, and its other callers act on a
 * link that was assessed first.
 */
function suffixTable(load = true): SuffixTable | null {
  if (tableLoaded.value) return table;
  if (load) void loadPublicSuffixTable();
  provisionalAnswers += 1;
  return null;
}


export type ExternalLinkRisk =
  | 'display_host_mismatch'
  | 'insecure_http'
  | 'embedded_credentials'
  | 'ip_address'
  | 'punycode_hostname'
  | 'unusual_port'
  | 'local_hostname';

export interface ExternalLinkAssessment {
  allowed: boolean;
  href: string;
  hostname: string;
  displayedHostname?: string;
  requiresConfirmation: boolean;
  risks: ExternalLinkRisk[];
}

export interface MailtoTarget {
  href: string;
  recipients: string[];
  subject: string;
  body: string;
}

export interface LinkAnchorRect {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export interface LinkInteraction {
  href: string;
  label: string;
  anchor: LinkAnchorRect;
}

const EMAIL_ADDRESS_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** RFC 6068: `+` is a plus sign here, not the form encoding's space. */
function mailtoField(search: string, name: string): string | null {
  for (const pair of search.replace(/^\?/, '').split('&')) {
    const separator = pair.indexOf('=');
    const key = separator < 0 ? pair : pair.slice(0, separator);
    if (decodeURIComponent(key).toLowerCase() !== name) continue;
    return separator < 0 ? '' : decodeURIComponent(pair.slice(separator + 1));
  }
  return null;
}

export function parseMailtoLink(href: string): MailtoTarget | null {
  try {
    const parsed = new URL(href.trim());
    if (parsed.protocol !== 'mailto:') return null;
    const recipients = decodeURIComponent(parsed.pathname)
      .split(',')
      .map((value) => value.trim())
      .filter((value) => EMAIL_ADDRESS_PATTERN.test(value));
    if (recipients.length === 0) return null;
    const subject = (mailtoField(parsed.search, 'subject') ?? '')
      .replace(/[\r\n]+/g, ' ')
      .trim()
      .slice(0, 998);
    const body = (mailtoField(parsed.search, 'body') ?? '').slice(0, 250_000);
    return {
      href: parsed.toString(),
      recipients,
      subject,
      body,
    };
  } catch {
    return null;
  }
}

function parseHttpUrl(value: string): URL | null {
  try {
    const parsed = new URL(value.trim());
    return parsed.protocol === 'https:' || parsed.protocol === 'http:' ? parsed : null;
  } catch {
    return null;
  }
}

/**
 * Google Calendar and other Google products wrap outbound links in
 * `https://www.google.com/url?q=...`. Treating the wrapper as the destination
 * produces a false display-host mismatch and leaves a tracking redirect in
 * the click path. Only unwrap the exact HTTPS endpoint we know; similarly
 * shaped URLs on other hosts must keep being assessed as written.
 */
function unwrapKnownRedirect(parsed: URL): URL {
  const isGoogleRedirect = parsed.protocol === 'https:'
    && (parsed.hostname.toLowerCase() === 'google.com'
      || parsed.hostname.toLowerCase() === 'www.google.com')
    && parsed.pathname === '/url'
    && !parsed.username
    && !parsed.password
    && !parsed.port;
  if (!isGoogleRedirect) return parsed;

  const target = parseHttpUrl(parsed.searchParams.get('q') ?? parsed.searchParams.get('url') ?? '');
  return target ?? parsed;
}

/**
 * What a reader takes the label for: NFKC folds fullwidth and other
 * compatibility forms, format characters (zero-width spaces, joiners, bidi
 * marks) are invisible and go, and the ideographic full stop reads as a dot.
 */
export function normalizeLinkLabel(label: string): string {
  return label
    .normalize('NFKC')
    .replace(/\p{Cf}/gu, '')
    .replace(/[\u3002\uFF61]/g, '.');
}

const URL_TOKEN = /https?:\/\/([^\s/?#<>"']+)[^\s<>"']*/giu;
// Not part of an address, a path or a longer name, so `user@bank.example` and `x/bank.example` are skipped.
const HOST_TOKEN = /(?<![\p{L}\p{N}@./_-])((?:[\p{L}\p{N}](?:[\p{L}\p{N}-]{0,61}[\p{L}\p{N}])?\.)+\p{L}{2,63})\.?(?![\p{L}\p{N}@-])/gu;
const WHOLE_LABEL_HOST = /^[^\s/?#]+(?::\d+)?(?:[/?#]\S*)?$/u;

interface DisplayedHost {
  /** As the reader sees it, for the prompt. */
  shown: string;
  /** IDNA form, comparable with a URL hostname. */
  ascii: string;
  lookalike: boolean;
}

function displayedHost(written: string): DisplayedHost | null {
  const shown = written.toLowerCase().replace(/^.*@/, '').replace(/:\d*$/, '').replace(/\.$/, '');
  try {
    const ascii = new URL(`https://${shown}`).hostname;
    // eslint-disable-next-line no-control-regex
    return ascii ? { shown, ascii, lookalike: /[^\x00-\x7f]/.test(shown) } : null;
  } catch {
    return null;
  }
}

/** A bare token inside running text counts once its suffix is a real one; before the table loads, any does. */
function knownSuffix(ascii: string): boolean {
  const suffixes = suffixTable();
  if (!suffixes) return true;
  const parsed = suffixes.parseHost(ascii, { allowPrivateDomains: true, extractHostname: false });
  return Boolean(parsed.isIcann || parsed.isPrivate);
}

/**
 * Every hostname the label shows, wherever it sits. URLs and a label that is
 * a host as a whole count with any suffix; a token inside running text needs
 * a known one, so "report.pdf" is not a host.
 */
function displayedHostnames(label: string): DisplayedHost[] {
  const normalized = normalizeLinkLabel(label).trim();
  const hosts = new Map<string, DisplayedHost>();
  const add = (written: string, requireKnownSuffix: boolean) => {
    const host = displayedHost(written);
    if (!host || hosts.has(host.ascii)) return;
    if (requireKnownSuffix && !knownSuffix(host.ascii)) return;
    hosts.set(host.ascii, host);
  };

  for (const match of normalized.matchAll(URL_TOKEN)) add(match[1]!, false);
  const text = normalized.replace(URL_TOKEN, ' ').trim();
  const whole = WHOLE_LABEL_HOST.test(text);
  for (const match of text.matchAll(HOST_TOKEN)) add(match[1]!, !whole);
  return [...hosts.values()];
}

function isIpAddress(hostname: string): boolean {
  return /^\d{1,3}(?:\.\d{1,3}){3}$/.test(hostname) || hostname.includes(':');
}

function hostnamesMatch(displayed: DisplayedHost, actual: string): boolean {
  if (displayed.ascii === actual) return true;
  // A label in another script that only looks like an ASCII domain never vouches for one.
  if (displayed.lookalike) return false;
  // Without the table, two different hostnames stay different.
  const suffixes = suffixTable();
  if (!suffixes) return false;
  const options = { allowPrivateDomains: true, extractHostname: false };
  const displayedDomain = suffixes.getDomain(displayed.ascii, options);
  const actualDomain = suffixes.getDomain(actual, options);
  return displayedDomain !== null && displayedDomain === actualDomain;
}

/**
 * The registrable (eTLD+1) domain of a hostname, the unit an "always open
 * without asking" decision can meaningfully cover: confirming `shop.example.com`
 * stands for `example.com`, because senders rotate subdomains freely. Hosts
 * without a registrable domain — IPs, localhost — fall back to themselves so
 * they stay individually addressable, as does a host the table cannot yet be
 * asked about: a narrower permission than intended, never a wider one.
 */
export function linkRegistrableDomain(hostname: string): string {
  const normalized = hostname.trim().toLowerCase();
  if (!normalized) return '';
  const suffixes = suffixTable(false);
  if (!suffixes) return normalized;
  return suffixes.getDomain(normalized, { allowPrivateDomains: true, extractHostname: false }) ?? normalized;
}

export function assessExternalLink(href: string, label = ''): ExternalLinkAssessment {
  const outer = parseHttpUrl(href);
  if (!outer) {
    return {
      allowed: false,
      href: '',
      hostname: '',
      requiresConfirmation: false,
      risks: [],
    };
  }

  const parsed = unwrapKnownRedirect(outer);

  const hostname = parsed.hostname.toLowerCase();
  const visibleHosts = displayedHostnames(label);
  const mismatch = visibleHosts.find((visible) => !hostnamesMatch(visible, hostname));
  const visibleHostname = (mismatch ?? visibleHosts[0])?.shown;
  const risks: ExternalLinkRisk[] = [];

  if (mismatch) {
    risks.push('display_host_mismatch');
  }
  if (parsed.protocol === 'http:') risks.push('insecure_http');
  if (parsed.username || parsed.password) risks.push('embedded_credentials');
  if (isIpAddress(hostname)) risks.push('ip_address');
  if (hostname.split('.').some((part) => part.startsWith('xn--'))) risks.push('punycode_hostname');
  if (parsed.port) risks.push('unusual_port');
  if (hostname === 'localhost' || hostname.endsWith('.localhost')) risks.push('local_hostname');

  const confirmationRisks = risks.filter((risk) => risk !== 'insecure_http');
  return {
    allowed: true,
    href: parsed.toString(),
    hostname,
    displayedHostname: visibleHostname,
    requiresConfirmation: confirmationRisks.length > 0,
    risks,
  };
}
