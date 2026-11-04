import { t } from '@/i18n/runtime';
import { formatMoment } from '@/utils/listTime';
import type { CertificateRejection, CertificateRejectionReason } from '@/utils/certificateRejection';

/**
 * Wording for a rejected certificate. The keys are spelled out per reason so
 * `i18n:check` sees every one of them; a template literal would hide them.
 */
const TITLES: Record<CertificateRejectionReason, string> = {
  expired: 'certificates.banner.title.expired',
  notYetValid: 'certificates.banner.title.notYetValid',
  nameMismatch: 'certificates.banner.title.nameMismatch',
  unknownIssuer: 'certificates.banner.title.unknownIssuer',
  other: 'certificates.banner.title.other',
};

const DETAILS: Record<CertificateRejectionReason, string> = {
  expired: 'certificates.banner.detail.expired',
  notYetValid: 'certificates.banner.detail.notYetValid',
  nameMismatch: 'certificates.banner.detail.nameMismatch',
  unknownIssuer: 'certificates.banner.detail.unknownIssuer',
  other: 'certificates.banner.detail.other',
};

const SHORT: Record<CertificateRejectionReason, string> = {
  expired: 'certificates.short.expired',
  notYetValid: 'certificates.short.notYetValid',
  nameMismatch: 'certificates.short.nameMismatch',
  unknownIssuer: 'certificates.short.unknownIssuer',
  other: 'certificates.short.other',
};

/** "today, 12:06" for today, otherwise "4 Nov, 12:06". Unknown stays unknown. */
export function formatCertificateMoment(value: string | null | undefined): string {
  return formatMoment(value) ?? t('certificates.dialog.unknown');
}

function moments(rejection: CertificateRejection) {
  return {
    host: rejection.host,
    since: formatCertificateMoment(rejection.notAfter),
    from: formatCertificateMoment(rejection.notBefore),
  };
}

export function certificateTitle(rejection: CertificateRejection, account: string): string {
  return t(TITLES[rejection.reason], { account });
}

export function certificateDetail(rejection: CertificateRejection): string {
  return t(DETAILS[rejection.reason], moments(rejection));
}

/** The one-line form for a tooltip, the phone banner and the IT message. */
export function certificateShort(rejection: CertificateRejection): string {
  return t(SHORT[rejection.reason], moments(rejection));
}

/** Ready to paste into a ticket or a mail to whoever runs the server. */
export function certificateNote(rejection: CertificateRejection): string {
  const unknown = t('certificates.dialog.unknown');
  return t('certificates.dialog.noteBody', {
    host: rejection.host,
    port: rejection.port || unknown,
    protocol: rejection.protocol.toUpperCase(),
    problem: certificateShort(rejection),
    subject: rejection.subject ?? unknown,
    issuer: rejection.issuer ?? unknown,
    notBefore: rejection.notBefore ?? unknown,
    notAfter: rejection.notAfter ?? unknown,
    fingerprint: rejection.fingerprint,
  });
}
