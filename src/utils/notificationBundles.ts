export type NotificationBundleIdentity =
  | readonly ['test-build', string, string]
  | readonly ['job-status', string]
  | readonly ['cron-output', string];

/** Recognize repeated machine events while retaining the resource they concern. */
export function detectNotificationBundle(senderEmail: string, subject: string): NotificationBundleIdentity | null {
  // Shell commands are case-sensitive and quoted whitespace can be meaningful.
  // Keep their complete subject before applying any display normalization.
  if (/^Cron <[^<>\r\n]+> \S/.test(subject)) return ['cron-output', subject];

  const normalized = subject.normalize('NFC').replace(/\s+/g, ' ').trim();
  if (/^testflight(?:[-_]no[-_]reply)?@(?:email\.)?apple\.com$/.test(senderEmail)) {
    const build = normalized.match(/^(.+?) \d+(?:\.\d+){1,3}(?:[-+][\w.-]+)? \(\d+(?:\.\d+)*\) for (iOS|iPadOS|macOS|tvOS|visionOS|watchOS) is now available to test\.?$/i);
    if (build) return ['test-build', build[1]!, build[2]!.toLowerCase()];
  }

  if (/^(?:no[-_]?reply|notifications?)@cron-job\.org$/.test(senderEmail)) {
    const job = normalized.match(/^Cron[- ]Job (?:erfolgreich|fehlgeschlagen|successful|failed):\s*(.+)$/i);
    if (job) return ['job-status', job[1]!];
  }
  return null;
}
