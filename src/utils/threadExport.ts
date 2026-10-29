import type { Message } from '@/types/mail';

export type ThreadExportScope = 'latest' | 'conversation';
export type ThreadExportFormat = 'markdown' | 'text';

export interface ThreadExportLabels {
  from: string;
  to: string;
  cc: string;
  date: string;
}

export interface ThreadExportRenderOptions {
  labels: ThreadExportLabels;
  formatDate: (date: Date) => string;
}

export interface ThreadExportDescriptor {
  filename: string;
  content: string;
  mime: string;
  extension: string;
}

/** Keep the store's order and its latest-message semantics for exports. */
export function selectThreadExportMessages(
  messages: readonly Message[],
  scope: ThreadExportScope,
): readonly Message[] {
  return scope === 'latest' ? messages.slice(-1) : messages;
}

export function formatThreadParticipantLine(
  message: Message,
  field: 'from' | 'to' | 'cc',
): string {
  const participants = field === 'from' ? [message.from] : message[field] ?? [];
  return participants
    .map((participant) => participant.name
      ? `${participant.name} <${participant.email}>`
      : participant.email)
    .join(', ');
}

export function renderThreadMarkdown(
  subject: string,
  messages: readonly Message[],
  scope: ThreadExportScope,
  options: ThreadExportRenderOptions,
): string {
  const sections = selectThreadExportMessages(messages, scope).map((message) => {
    const cc = formatThreadParticipantLine(message, 'cc');
    return [
      `## ${formatThreadParticipantLine(message, 'from')} · ${options.formatDate(new Date(message.timestamp))}`,
      '',
      `**${options.labels.from}:** ${formatThreadParticipantLine(message, 'from')}  `,
      `**${options.labels.to}:** ${formatThreadParticipantLine(message, 'to')}  `,
      ...(cc ? [`**${options.labels.cc}:** ${cc}  `] : []),
      '',
      message.bodyText ?? '',
    ].join('\n');
  });
  return [`# ${subject}`, '', ...sections].join('\n\n').trimEnd() + '\n';
}

export function renderThreadText(
  subject: string,
  messages: readonly Message[],
  scope: ThreadExportScope,
  options: ThreadExportRenderOptions,
): string {
  const sections = selectThreadExportMessages(messages, scope).map((message) => {
    const cc = formatThreadParticipantLine(message, 'cc');
    return [
      `${options.labels.from}: ${formatThreadParticipantLine(message, 'from')}`,
      `${options.labels.to}: ${formatThreadParticipantLine(message, 'to')}`,
      ...(cc ? [`${options.labels.cc}: ${cc}`] : []),
      `${options.labels.date}: ${options.formatDate(new Date(message.timestamp))}`,
      '',
      message.bodyText ?? '',
    ].join('\n');
  });
  return [subject, '='.repeat(Math.min(72, Math.max(8, subject.length))), '', ...sections]
    .join('\n\n')
    .trimEnd() + '\n';
}

export function sanitizeThreadFilename(value: string, fallback: string): string {
  // Filenames must omit ASCII controls on every supported filesystem.
  // oxlint-disable-next-line no-control-regex
  const cleaned = value.replace(/[<>:"/\\|?*\u0000-\u001F]/g, ' ').replace(/\s+/g, ' ').trim();
  return (cleaned || fallback).slice(0, 90);
}

export function buildThreadExportDescriptor(
  format: ThreadExportFormat,
  subject: string,
  messages: readonly Message[],
  options: ThreadExportRenderOptions & { filenameFallback: string },
): ThreadExportDescriptor {
  const base = sanitizeThreadFilename(subject, options.filenameFallback);
  if (format === 'markdown') {
    return {
      filename: `${base}.md`,
      content: renderThreadMarkdown(subject, messages, 'conversation', options),
      mime: 'text/markdown',
      extension: 'md',
    };
  }
  return {
    filename: `${base}.txt`,
    content: renderThreadText(subject, messages, 'conversation', options),
    mime: 'text/plain',
    extension: 'txt',
  };
}
